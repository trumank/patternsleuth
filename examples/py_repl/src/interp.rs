//! Drives a persistent [`code.InteractiveInterpreter`] on the engine thread via
//! an embedded RustPython interpreter. Each request is a complete chunk of
//! source; we run it with `single` mode semantics (bare expressions print their
//! repr) and return captured stdout+stderr.
//!
//! Everything lives in a thread-local: RustPython's `Interpreter` / `Scope` /
//! `PyObjectRef` are all `Rc`-based (`!Send`), and all execution happens on the
//! one engine/main thread (the TCP workers only ship source strings over a
//! channel). The interpreter is created lazily on the first [`run`], which
//! happens inside the engine tick.
//!
//! [`with_vm`] re-enters the same interpreter so native engine callbacks
//! (UFunction hooks, begin-play, trace) can call back into Python. RustPython's
//! `enter_vm` is nestable, so this is safe even while a REPL `run` is on the
//! stack above us.
//!
//! ## Async runs
//! A frame that defines `async def main()` is driven as a coroutine instead of
//! replied to synchronously. `__ps_run` hands the coroutine to a tiny tick-
//! driven event loop (`_aio`, built on `ue.on_tick`) and returns `None`, which
//! [`run`] reports as [`server::Outcome::Pending`]. The loop resumes the
//! coroutine across ticks — so the game thread keeps ticking (and generating)
//! between `await`s — streaming each step's stdout via `ue._emit` and sending
//! the coroutine's return value via `ue._complete` when it finishes.

use std::cell::OnceCell;
use std::mem::ManuallyDrop;

use rustpython_vm::builtins::PyTuple;
use rustpython_vm::compiler::Mode;
use rustpython_vm::scope::Scope;
use rustpython_vm::{Interpreter, Settings, VirtualMachine};

use crate::{bindings, server};

const BOOTSTRAP: &str = r#"
import code, contextlib, heapq, io, sys, traceback as _tb

CO_COROUTINE = 0x80   # code object flag identifying `async def`

# Reply stream tags, mirroring server::TAG_*. stdout and stderr are captured and
# shipped separately so a client can route them independently.
_STDOUT = 0
_STDERR = 2

class _Buf:
    def __init__(self):
        self.parts = []
    def write(self, s):
        self.parts.append(s)
    def flush(self):
        pass

class _Interp(code.InteractiveInterpreter):
    def __init__(self, env):
        super().__init__(env)
        self._buf = None
    def write(self, data):
        # Tracebacks land here. During a run `_buf` is set; outside one we have
        # nowhere to go (no `stdio` feature), so drop it rather than raise.
        buf = self._buf
        if buf is not None:
            buf.write(data)

_interp = _Interp(globals())

class _Safe:
    # Base sys.stdout/stderr. Inside a run/step, redirect_stdout points the real
    # stream at a _Buf; outside one (e.g. a stray print from a detached hook on
    # the game thread) writes route to _interp._buf if set, else drop -- so a
    # rogue print can never raise and kill an engine callback.
    def write(self, s):
        buf = _interp._buf
        if buf is not None:
            buf.write(s)
    def flush(self):
        pass

sys.stdout = _Safe()
sys.stderr = _Safe()

# ── async runtime ────────────────────────────────────────────────────────────

class Timeout(Exception):
    pass

class Cancelled(Exception):
    pass

class _HookEvent:
    # Detached snapshot of a hook firing (the live param buffer is gone by the
    # time the awaiting coroutine resumes). Read-only: no original()/mutation.
    __slots__ = ('this', 'args')
    def __init__(self, this, args):
        self.this = this
        self.args = args
    def __repr__(self):
        return '<HookEvent this=%r args=%r>' % (self.this, self.args)

class _Future:
    __slots__ = ('_done', '_val', '_exc', '_cbs')
    def __init__(self):
        self._done = False
        self._val = None
        self._exc = None
        self._cbs = []
    def done(self):
        return self._done
    def resolve(self, val=None):
        if self._done:
            return
        self._done = True
        self._val = val
        cbs, self._cbs = self._cbs, []
        for cb in cbs:
            _aio._enqueue(cb, self)
    def fail(self, exc):
        if self._done:
            return
        self._done = True
        self._exc = exc
        cbs, self._cbs = self._cbs, []
        for cb in cbs:
            _aio._enqueue(cb, self)
    def add_done_callback(self, cb):
        if self._done:
            _aio._enqueue(cb, self)
        else:
            self._cbs.append(cb)
    def result(self):
        if self._exc is not None:
            raise self._exc
        return self._val
    def __await__(self):
        if not self._done:
            yield self
        return self.result()

def _emit_streams(token, out, err):
    # Ship whatever each stream accumulated this step, each on its own tag.
    if token is None:
        return
    if out:
        ue._emit(token, _STDOUT, out)
    if err:
        ue._emit(token, _STDERR, err)

class _Task:
    __slots__ = ('coro', 'token', '_pre')
    def __init__(self, coro, token, pre=None):
        self.coro = coro
        self.token = token
        self._pre = pre   # (stdout, stderr) captured before the first await, or None
    def _run(self, send_val=None, send_exc=None):
        if self._pre is not None:
            _emit_streams(self.token, self._pre[0], self._pre[1])
            self._pre = None
        out, err = _Buf(), _Buf()
        # Tracebacks (via _Interp.write) belong on stderr.
        prev, _interp._buf = _interp._buf, err
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                try:
                    if send_exc is not None:
                        fut = self.coro.throw(send_exc)
                    else:
                        fut = self.coro.send(send_val)
                except StopIteration as e:
                    self._finish(out, err, result=e.value)
                    return
                except BaseException:
                    err.write(_tb.format_exc())
                    self._finish(out, err)
                    return
        finally:
            _interp._buf = prev
        _emit_streams(self.token, ''.join(out.parts), ''.join(err.parts))
        if isinstance(fut, _Future):
            fut.add_done_callback(self._resume)
        else:
            # awaited something we don't understand -> just resume next pump
            _aio._enqueue(self._resume, None)
    def _resume(self, fut):
        if fut is None:
            self._run(None)
        elif fut._exc is not None:
            self._run(send_exc=fut._exc)
        else:
            self._run(send_val=fut._val)
    def _finish(self, out, err, result=None):
        if self.token is None:
            return
        _emit_streams(self.token, ''.join(out.parts), ''.join(err.parts))
        if result is None:
            res = ''
        elif isinstance(result, str):
            res = result
        else:
            res = repr(result)
        ue._complete(self.token, res)

class _Aio:
    __slots__ = ('_ready', '_tick', '_timers', '_clock', '_seq', '_next')
    def __init__(self):
        self._ready = []     # [(cb, arg)] resumes/callbacks to run next pump
        self._tick = None    # ue.on_tick handle (installed lazily)
        self._timers = []    # heap of (deadline, seq, future)
        self._clock = 0.0    # accumulated tick dt
        self._seq = 0
        self._next = []      # next-tick futures
    def _ensure(self):
        if self._tick is None:
            self._tick = ue.on_tick(self._pump)
    def _enqueue(self, cb, arg):
        self._ready.append((cb, arg))
        self._ensure()
    def _pump(self, world, dt):
        self._clock += dt
        nxt, self._next = self._next, []
        for f in nxt:
            f.resolve(dt)
        while self._timers and self._timers[0][0] <= self._clock:
            heapq.heappop(self._timers)[2].resolve(None)
        batch, self._ready = self._ready, []
        for cb, arg in batch:
            try:
                cb(arg)
            except BaseException:
                pass   # task errors are handled in _Task._run; ignore stray cbs
        if not self._ready and not self._timers and not self._next:
            t, self._tick = self._tick, None
            if t is not None:
                t.remove()
    def spawn(self, coro, token, pre=''):
        task = _Task(coro, token, pre)
        self._enqueue(lambda _a: task._run(None), None)
        return task
    def sleep(self, seconds):
        f = _Future()
        self._seq += 1
        heapq.heappush(self._timers, (self._clock + seconds, self._seq, f))
        self._ensure()
        return f
    def next_tick(self):
        f = _Future()
        self._next.append(f)
        self._ensure()
        return f
    def deadline(self, fut, seconds, on_timeout):
        tf = self.sleep(seconds)
        def fire(_):
            if not fut._done:
                try:
                    on_timeout()
                except BaseException:
                    pass
                fut.fail(Timeout())
        tf.add_done_callback(fire)

_aio = _Aio()

def _safe_unhook(cls, func):
    try:
        ue.unhook(cls, func)
    except Exception:
        pass

def _safe_remove(holder):
    if holder:
        try:
            holder[0].remove()
        except Exception:
            pass

async def _sleep(seconds, real=False):
    # `real` (wall-clock) not wired yet; tick-dt based, so it respects pause.
    return await _aio.sleep(seconds)

async def _next_tick():
    return await _aio.next_tick()

async def _hook_once(cls, func, where=None, timeout=None):
    f = _Future()
    def cb(this, p):
        if where is not None and not where(this, p):
            return
        _safe_unhook(cls, func)
        if not f._done:
            f.resolve(_HookEvent(this, list(p.args)))
    ue.hook(cls, func, cb)
    if timeout is not None:
        _aio.deadline(f, timeout, lambda: _safe_unhook(cls, func))
    return await f

async def _spawned(cls, where=None, timeout=None):
    f = _Future()
    holder = []
    def cb(actor):
        if where is not None and not where(actor):
            return
        if f._done:
            return
        _safe_remove(holder)
        f.resolve(actor)
    holder.append(ue.on_begin_play(cls, cb))
    if timeout is not None:
        _aio.deadline(f, timeout, lambda: _safe_remove(holder))
    return await f

# Surface the awaitables on the `ue` module to match the rest of the API.
ue.sleep = _sleep
ue.next_tick = _next_tick
ue.hook_once = _hook_once
ue.spawned = _spawned
ue.Timeout = Timeout
ue.Cancelled = Cancelled

# ── request entry point ──────────────────────────────────────────────────────

def __ps_run(token, source):
    out, err = _Buf(), _Buf()
    _interp._buf = err   # tracebacks (via _Interp.write) go to stderr
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                more = _interp.runsource(source, '<repl>', 'single')
            except SystemExit:
                err.write('(SystemExit)\n')
                more = False
    finally:
        _interp._buf = None
    if more:
        return ('', '', '<incomplete>\n')
    g = globals()
    main = g.get('main')
    if main is not None and getattr(main, '__code__', None) is not None \
            and (main.__code__.co_flags & CO_COROUTINE):
        # async run: drive it on the loop. Any top-level output becomes the
        # first streamed chunk; returning None tells Rust this run is Pending.
        del g['main']
        _aio.spawn(main(), token, (''.join(out.parts), ''.join(err.parts)))
        return None
    # sync run: (stdout, stderr, result). result is empty -- single-mode echoes
    # expression values onto stdout itself, so there's nothing extra to carry.
    return (''.join(out.parts), ''.join(err.parts), '')
"#;

struct ReplState {
    interp: Interpreter,
    scope: Scope,
}

thread_local! {
    // `ManuallyDrop` so the interpreter is *never* torn down at thread exit.
    // We're a DLL injected into the game: the only time this thread-local would
    // drop is at process shutdown, and tearing the RustPython VM down there runs
    // `PyObjectRef` (Rc) destructors that reach into *other* thread-locals (the
    // hook/begin-play/trace registries) which may already be destroyed — that's
    // the classic "cannot access a TLS value during or after destruction" panic.
    // `OnceCell<ManuallyDrop<_>>` doesn't need drop, so no destructor is
    // registered and the OS just reclaims the memory. Same policy as
    // `bindings`' REGISTRIES and `server`'s SERVER thread-locals.
    static STATE: OnceCell<ManuallyDrop<ReplState>> = const { OnceCell::new() };
}

fn build() -> ReplState {
    let mut settings = Settings::default();
    // No filesystem stdlib / site lookups — everything is frozen into the binary.
    settings.import_site = false;
    // Do NOT let the embedded VM install process signal handlers. RustPython
    // imports `_signal` unconditionally at startup, and with this enabled it
    // calls `libc::signal(SIGINT, ...)` — which on Windows registers a console
    // control handler that swallows Ctrl+C. In an injected DLL that hijacks the
    // *host game's* Ctrl+C, so the game stops exiting on Ctrl+C and instead
    // raises a KeyboardInterrupt that goes nowhere. We're a guest; leave the
    // process's signal disposition to the game.
    settings.install_signal_handlers = false;

    let builder = Interpreter::builder(settings);
    // Native (Rust) stdlib modules — `_contextvars`, `_io`'s friends, etc.
    let stdlib_defs = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
    let ue_def = bindings::module_def(&builder.ctx);
    let interp = builder
        .add_native_modules(&stdlib_defs)
        .add_native_module(ue_def)
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
        .build();

    let scope = interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let init = || -> rustpython_vm::PyResult<()> {
            let ue = vm.import("ue", 0)?;
            scope.globals.set_item("ue", ue, vm)?;
            scope
                .globals
                .set_item("__name__", vm.new_pyobj("__ps_repl__"), vm)?;
            let code = vm
                .compile(BOOTSTRAP, Mode::Exec, "<bootstrap>".to_owned())
                .map_err(|e| vm.new_syntax_error(&e, Some(BOOTSTRAP)))?;
            vm.run_code_obj(code, scope.clone())?;
            Ok(())
        };
        if let Err(e) = init() {
            let mut tb = String::new();
            let _ = vm.write_exception(&mut tb, &e);
            tracing::error!("py_repl bootstrap failed:\n{tb}");
        }
        scope
    });

    ReplState { interp, scope }
}

/// Run a closure with the embedded VM, re-entering the interpreter. Returns
/// `None` (closure not run) if the interpreter hasn't been initialized yet —
/// which can't happen for hook/begin-play callbacks, since those can only be
/// registered from REPL code that runs *after* init.
pub fn with_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> Option<R> {
    STATE.with(|cell| cell.get().map(|st| st.interp.enter(|vm| f(vm))))
}

/// Run one request. `token` identifies this request for deferred/streamed
/// replies: a synchronous run returns [`server::Outcome::Done`] with its
/// captured stdout/stderr; an `async def main()` run starts a coroutine (which
/// will later `ue._emit`/`ue._complete` against `token`) and returns
/// [`server::Outcome::Pending`].
pub fn run(token: u64, source: &str) -> server::Outcome {
    STATE.with(|cell| {
        let st = cell.get_or_init(|| ManuallyDrop::new(build()));
        st.interp.enter(|vm| {
            let Ok(run_fn) = st.scope.globals.get_item("__ps_run", vm) else {
                return done_err("(py_repl not initialized)\n");
            };
            match run_fn.call((vm.ctx.new_int(token), source.to_owned()), vm) {
                // `None` => the run went async and kept the token (Pending).
                Ok(v) if vm.is_none(&v) => server::Outcome::Pending,
                // Otherwise `__ps_run` hands back a (stdout, stderr, result) tuple.
                Ok(v) => match v.downcast::<PyTuple>() {
                    Ok(t) => {
                        let items = t.as_slice();
                        let field = |i: usize| {
                            items
                                .get(i)
                                .and_then(|o| o.clone().try_into_value::<String>(vm).ok())
                                .unwrap_or_default()
                        };
                        server::Outcome::Done {
                            stdout: field(0),
                            stderr: field(1),
                            result: field(2),
                        }
                    }
                    Err(_) => done_err("py_repl: malformed run result\n"),
                },
                Err(e) => {
                    // `__ps_run` catches user exceptions itself; reaching here means
                    // the REPL harness failed. Render the traceback onto stderr.
                    let mut msg = String::new();
                    let _ = vm.write_exception(&mut msg, &e);
                    done_err(&format!("py_repl: {msg}\n"))
                }
            }
        })
    })
}

/// A finished run carrying only a stderr diagnostic (harness-level failure).
fn done_err(msg: &str) -> server::Outcome {
    server::Outcome::Done {
        stdout: String::new(),
        stderr: msg.to_string(),
        result: String::new(),
    }
}
