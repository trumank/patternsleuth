//! TCP listener. Each connection sends length-prefixed UTF-8 source strings;
//! we reply with one or more length-prefixed, *tagged* frames.
//!
//! Wire reply format (per request): a sequence of frames
//!     [u32 LE total_len][u8 tag][UTF-8 bytes]
//! where `total_len` counts the tag byte. A request produces zero or more
//! `TAG_STDOUT` / `TAG_STDERR` frames (the captured/streamed output streams,
//! kept separate) followed by exactly one terminating `TAG_RESULT` frame (the
//! run's result value, usually empty). Clients "read frames until TAG_RESULT",
//! routing STDOUT/STDERR to their own streams.
//!
//! The listener is a worker thread, but execution happens on the main thread
//! during [`drain_and_execute`] (called from the engine tick). For a synchronous
//! run the worker blocks on `reply_rx.recv()` until the main thread sends the
//! result. For an *async* run, [`drain_and_execute`] parks the reply sender in a
//! per-token registry; the running coroutine later streams chunks via [`emit`]
//! and finishes via [`complete`], each of which sends a frame the still-blocked
//! worker writes out. The worker loops until it sees the `TAG_RESULT` frame.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;
use std::sync::mpsc;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, bounded};

pub const TAG_STDOUT: u8 = 0;
pub const TAG_RESULT: u8 = 1;
pub const TAG_STDERR: u8 = 2;

/// What running one request produced. `Done` replies immediately, sending the
/// captured `stdout`/`stderr` (each as its own frame, when non-empty) then the
/// `result` terminator. `Pending` means the run started an async task that will
/// stream + complete later, so its reply sender is parked under the token.
pub enum Outcome {
    Done {
        stdout: String,
        stderr: String,
        result: String,
    },
    Pending,
}

pub struct Pending {
    pub source: String,
    pub reply: mpsc::Sender<(u8, String)>,
}

/// The cross-thread handoff point: listener threads send `Pending` requests,
/// the engine thread receives them in [`drain_and_execute`]. The only
/// process-wide global here — everything else is engine-thread-local.
static QUEUE: OnceLock<Receiver<Pending>> = OnceLock::new();

/// Engine-thread request state: the token counter naming each run, and the
/// reply senders for in-flight *async* runs keyed by token (parked by
/// `drain_and_execute`, drained by `emit`/`complete` — all engine thread).
struct ServerState {
    next_token: Cell<u64>,
    replies: RefCell<HashMap<u64, mpsc::Sender<(u8, String)>>>,
}

thread_local! {
    // ManuallyDrop: game-thread TLS is never torn down in this DLL — see the
    // policy on `bindings`' REGISTRIES. (Dropping the parked senders would be
    // harmless today, but one uniform rule beats auditing each case.)
    static SERVER: std::mem::ManuallyDrop<ServerState> =
        std::mem::ManuallyDrop::new(ServerState {
            next_token: Cell::new(1),
            replies: RefCell::new(HashMap::new()),
        });
}

pub fn spawn(port: u16) -> Result<()> {
    let (tx, rx) = bounded::<Pending>(64);
    QUEUE
        .set(rx)
        .map_err(|_| anyhow::anyhow!("py_repl server already initialized"))?;
    std::thread::spawn(move || {
        if let Err(e) = listen(port, tx) {
            tracing::error!("py_repl listener: {e:#}");
        }
    });
    Ok(())
}

/// Run each queued request. `run(token, source)` returns either a finished
/// `Done(output)` (sent now) or `Pending` (the run kept `token` and will stream
/// + complete asynchronously; we park its reply sender under that token).
pub fn drain_and_execute(mut run: impl FnMut(u64, &str) -> Outcome) {
    let Some(rx) = QUEUE.get() else { return };
    while let Ok(req) = rx.try_recv() {
        let token = SERVER.with(|s| {
            let t = s.next_token.get();
            s.next_token.set(t + 1);
            t
        });
        match run(token, &req.source) {
            Outcome::Done {
                stdout,
                stderr,
                result,
            } => {
                if !stdout.is_empty() {
                    let _ = req.reply.send((TAG_STDOUT, stdout));
                }
                if !stderr.is_empty() {
                    let _ = req.reply.send((TAG_STDERR, stderr));
                }
                let _ = req.reply.send((TAG_RESULT, result));
            }
            Outcome::Pending => {
                SERVER.with(|s| s.replies.borrow_mut().insert(token, req.reply));
            }
        }
    }
}

/// Stream a chunk of output for an in-flight async run on the given stream tag
/// (`TAG_STDOUT` or `TAG_STDERR`). No-op if the token is unknown (already
/// completed) or its client has gone away.
pub fn emit(token: u64, tag: u8, text: String) {
    SERVER.with(|s| {
        let dead = match s.replies.borrow().get(&token) {
            Some(tx) => tx.send((tag, text)).is_err(),
            None => false,
        };
        if dead {
            s.replies.borrow_mut().remove(&token);
        }
    });
}

/// Finish an in-flight async run: send the terminating result frame and drop
/// the parked sender (which lets the worker thread move on to the next request).
pub fn complete(token: u64, text: String) {
    SERVER.with(|s| {
        if let Some(tx) = s.replies.borrow_mut().remove(&token) {
            let _ = tx.send((TAG_RESULT, text));
        }
    });
}

fn listen(port: u16, tx: Sender<Pending>) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    tracing::info!("py_repl listening on 127.0.0.1:{port}");
    for stream in listener.incoming() {
        let stream = stream?;
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(stream, tx) {
                tracing::warn!("py_repl client: {e:#}");
            }
        });
    }
    Ok(())
}

fn handle(stream: TcpStream, tx: Sender<Pending>) -> Result<()> {
    let mut reader = stream.try_clone()?;
    let mut writer = stream;
    loop {
        let mut len_buf = [0u8; 4];
        match reader.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        let source = String::from_utf8(buf)?;

        let (reply_tx, reply_rx) = mpsc::channel::<(u8, String)>();
        tx.send(Pending {
            source,
            reply: reply_tx,
        })?;

        // Write frames until the terminating TAG_RESULT. A sync run sends one;
        // an async run streams chunks first. recv() errors only if the sender is
        // dropped without completing, which we treat as end-of-response.
        loop {
            let (tag, data) = match reply_rx.recv() {
                Ok(frame) => frame,
                Err(_) => break,
            };
            let bytes = data.as_bytes();
            let frame_len = (1 + bytes.len()) as u32;
            writer.write_all(&frame_len.to_le_bytes())?;
            writer.write_all(&[tag])?;
            writer.write_all(bytes)?;
            writer.flush()?;
            if tag == TAG_RESULT {
                break;
            }
        }
    }
}
