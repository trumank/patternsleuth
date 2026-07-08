use futures_util::{SinkExt, StreamExt};
use libue as ue;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::debugger::{self, DbgEvent, DebuggerChannels, GameRequest, ResumeCmd};

pub async fn run_server(mut channels: DebuggerChannels) {
    let listener = match TcpListener::bind("127.0.0.1:9222").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind CDP server: {e}");
            return;
        }
    };
    tracing::info!("CDP server listening on ws://127.0.0.1:9222");
    tracing::info!("Open: devtools://devtools/bundled/js_app.html?ws=localhost:9222");

    loop {
        let Ok((stream, addr)) = listener.accept().await else {
            continue;
        };
        tracing::info!("CDP connection from {addr}");

        if let Err(e) = handle_connection(stream, &mut channels).await {
            tracing::error!("CDP connection error: {e}");
        }
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    channels: &mut DebuggerChannels,
) -> anyhow::Result<()> {
    // Peek to see if it's HTTP or WebSocket
    let mut buf = [0u8; 4];
    stream.peek(&mut buf).await?;

    // Check for "GET " prefix (HTTP request)
    if &buf == b"GET " {
        // Read the full HTTP request to check the path
        let mut peek_buf = vec![0u8; 1024];
        let n = stream.peek(&mut peek_buf).await?;
        let request = String::from_utf8_lossy(&peek_buf[..n]);

        if request.contains("/json/version") {
            let response = json!({
                "Browser": "cdpdbg/0.1.0",
                "Protocol-Version": "1.3",
                "V8-Version": "0.0.0",
                "webSocketDebuggerUrl": "ws://localhost:9222/devtools/page/1"
            });
            let body = serde_json::to_string(&response)?;
            let http = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            use tokio::io::AsyncWriteExt;
            let mut stream = stream;
            stream.write_all(http.as_bytes()).await?;
            return Ok(());
        } else if request.contains("/json") && !request.contains("/devtools") {
            let state = debugger::shared();
            let response = json!([{
                "description": "",
                "devtoolsFrontendUrl": "devtools://devtools/bundled/js_app.html?ws=localhost:9222",
                "id": "1",
                "title": format!("cdpdbg ({} scripts)", state.scripts.lock().len()),
                "type": "page",
                "url": "kismet://game",
                "webSocketDebuggerUrl": "ws://localhost:9222/devtools/page/1"
            }]);
            let body = serde_json::to_string(&response)?;
            let http = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            use tokio::io::AsyncWriteExt;
            let mut stream = stream;
            stream.write_all(http.as_bytes()).await?;
            return Ok(());
        }
    }

    // WebSocket upgrade
    let ws_stream = tokio_tungstenite::accept_async(stream).await?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let send = |msg: &Value| -> anyhow::Result<String> {
        let text = serde_json::to_string(msg)?;
        tracing::debug!(">> {text}");
        Ok(text)
    };

    tracing::info!("CDP WebSocket session established");

    let DebuggerChannels {
        event_rx,
        request_tx,
    } = channels;

    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                let Some(msg) = msg else { break };
                let msg = msg?;
                let Message::Text(text) = msg else { continue };
                tracing::debug!("<< {text}");
                let request: Value = serde_json::from_str(&text)?;

                let id = request.get("id").and_then(|v| v.as_i64());
                let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let params = request.get("params").cloned().unwrap_or(json!({}));

                let (result, events) = handle_cdp_method(method, &params, &request_tx).await;

                if let Some(id) = id {
                    let text = send(&json!({"id": id, "result": result}))?;
                    ws_tx.send(Message::Text(text)).await?;
                }
                for event in events {
                    let text = send(&event)?;
                    ws_tx.send(Message::Text(text)).await?;
                }
            }
            event = event_rx.recv() => {
                let Some(event) = event else { break };
                tracing::debug!("Received DbgEvent: {}", match &event {
                    DbgEvent::Paused { .. } => "Paused",
                    DbgEvent::ScriptParsed { .. } => "ScriptParsed",
                });
                for msg in handle_dbg_event(&event) {
                    let text = send(&msg)?;
                    ws_tx.send(Message::Text(text)).await?;
                }
            }
        }
    }

    Ok(())
}

async fn handle_cdp_method(
    method: &str,
    params: &Value,
    request_tx: &crossbeam_channel::Sender<GameRequest>,
) -> (Value, Vec<Value>) {
    let state = debugger::shared();
    let mut events = Vec::new();

    let result = match method {
        // ── Debugger ────────────────────────────────────────────────────────
        "Debugger.enable" => {
            // Emit scriptParsed for all scripts
            for (i, script) in state.scripts.lock().iter().enumerate() {
                events.push(json!({
                    "method": "Debugger.scriptParsed",
                    "params": {
                        "scriptId": script.script_id,
                        "url": script.url,
                        "startLine": 0,
                        "startColumn": 0,
                        "endLine": script.line_count,
                        "endColumn": 0,
                        "executionContextId": 1,
                        "hash": format!("{:x}", i),
                        "sourceMapURL": "",
                        "hasSourceURL": false,
                        "isModule": false,
                        "length": script.source.len(),
                    }
                }));
            }
            json!({"debuggerId": "cdpdbg-1"})
        }

        "Debugger.getScriptSource" => {
            let script_id = params
                .get("scriptId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let scripts = state.scripts.lock();
            let source = scripts
                .iter()
                .find(|s| s.script_id == script_id)
                .map(|s| s.source.as_str())
                .unwrap_or("");
            json!({"scriptSource": source})
        }

        "Debugger.getPossibleBreakpoints" => {
            let empty = json!({});
            let start = params.get("start").unwrap_or(&empty);
            let script_id = start.get("scriptId").and_then(|v| v.as_str()).unwrap_or("");
            let start_line = start
                .get("lineNumber")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let end = params.get("end").unwrap_or(&empty);
            let end_line = end
                .get("lineNumber")
                .and_then(|v| v.as_u64())
                .unwrap_or(u64::MAX) as usize;

            let scripts = state.scripts.lock();
            let locations: Vec<Value> = scripts
                .iter()
                .find(|s| s.script_id == script_id)
                .map(|s| {
                    s.functions
                        .iter()
                        .flat_map(|f| f.source_map.iter())
                        .filter(|m| m.line >= start_line && m.line <= end_line)
                        .map(|m| {
                            json!({
                                "scriptId": script_id,
                                "lineNumber": m.line,
                                "columnNumber": m.col,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({"locations": locations})
        }

        "Debugger.setBreakpointByUrl" => {
            let line = params
                .get("lineNumber")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let col = params
                .get("columnNumber")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("");

            let found = {
                let scripts = state.scripts.lock();
                scripts
                    .iter()
                    .enumerate()
                    .find(|(_, s)| s.url == url)
                    .map(|(idx, s)| (idx, s.script_id.clone()))
            };
            if let Some((idx, script_id)) = found {
                let bp_id = debugger::set_breakpoint(state, idx, line, col);
                json!({
                    "breakpointId": bp_id,
                    "locations": [{
                        "scriptId": script_id,
                        "lineNumber": line,
                        "columnNumber": col,
                    }]
                })
            } else {
                json!({"breakpointId": "", "locations": []})
            }
        }

        "Debugger.removeBreakpoint" => {
            let bp_id = params
                .get("breakpointId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            debugger::remove_breakpoint(state, bp_id);
            json!({})
        }

        "Debugger.resume" => {
            let _ = request_tx.send(GameRequest::Resume(ResumeCmd::Continue));
            events.push(json!({"method": "Debugger.resumed", "params": {}}));
            json!({})
        }

        "Debugger.stepOver" => {
            let _ = request_tx.send(GameRequest::Resume(ResumeCmd::StepOver));
            events.push(json!({"method": "Debugger.resumed", "params": {}}));
            json!({})
        }

        "Debugger.stepInto" => {
            let _ = request_tx.send(GameRequest::Resume(ResumeCmd::StepInto));
            events.push(json!({"method": "Debugger.resumed", "params": {}}));
            json!({})
        }

        "Debugger.stepOut" => {
            let _ = request_tx.send(GameRequest::Resume(ResumeCmd::StepOut));
            events.push(json!({"method": "Debugger.resumed", "params": {}}));
            json!({})
        }

        "Debugger.pause" => {
            let saved = debugger::install_step_gnatives(state);
            *state.step_state.lock() = Some(debugger::StepState {
                mode: debugger::StepMode::Into,
                origin_node: Default::default(),
                origin_depth: 0,
                origin_line: None,
                saved_gnatives: saved,
            });
            json!({})
        }

        "Debugger.setScriptSource" => {
            let script_id = params
                .get("scriptId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let source = params
                .get("scriptSource")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut scripts = state.scripts.lock();
            if let Some((script_idx, script)) = scripts
                .iter_mut()
                .enumerate()
                .find(|(_, s)| s.script_id == script_id)
            {
                let script_url = script.url.clone();
                let class_path = script.class_path.clone();
                match crate::scripts::set_script_source(script, source) {
                    Ok(()) => {
                        drop(scripts);
                        // Breakpoints on this script now reference stale
                        // offsets in the orphaned (leaked) bytecode — drop
                        // them rather than trying to patch back through the
                        // new byte buffer.
                        state
                            .breakpoints
                            .lock()
                            .retain(|bp| bp.script_idx != script_idx);
                        json!({"status": "Ok"})
                    }
                    Err(e) => {
                        tracing::warn!("setScriptSource failed for {}: {}", class_path, e.message);
                        let (line, col) = e
                            .span
                            .map(|(start, _)| byte_offset_to_line_col(source, start))
                            .unwrap_or((0, 0));
                        json!({
                            "status": "CompileError",
                            "exceptionDetails": {
                                "exceptionId": 1,
                                "text": e.message,
                                "lineNumber": line,
                                "columnNumber": col,
                                "scriptId": script_id,
                                "url": script_url,
                            }
                        })
                    }
                }
            } else {
                tracing::warn!("setScriptSource: script not found: {script_id}");
                json!({})
            }
        }

        "Debugger.setBreakpointsActive" => {
            // TODO: track active state
            json!({})
        }

        "Debugger.setPauseOnExceptions"
        | "Debugger.setAsyncCallStackDepth"
        | "Debugger.setBlackboxPatterns" => json!({}),

        "Debugger.evaluateOnCallFrame" => {
            // TODO
            json!({"result": {"type": "undefined"}})
        }

        // ── Runtime ─────────────────────────────────────────────────────────
        "Runtime.enable" => {
            events.push(json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": 1,
                        "origin": "kismet://game",
                        "name": "cdpdbg",
                    }
                }
            }));
            json!({})
        }

        "Runtime.getProperties" => {
            let object_id = params
                .get("objectId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = request_tx.send(GameRequest::ReadProperties {
                object_id: object_id.to_string(),
                resp: tx,
            });
            match rx.await {
                Ok(result) => json!({"result": result}),
                Err(_) => json!({"result": []}),
            }
        }

        "Runtime.evaluate" => {
            let expr = params
                .get("expression")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result_str = eval_repl(expr);
            json!({"result": {"type": "string", "value": result_str}})
        }

        "Runtime.compileScript" | "Runtime.callFunctionOn" | "Runtime.getHeapUsage" => {
            json!({"result": {"type": "undefined"}})
        }

        // ── Page / Target stubs ─────────────────────────────────────────────
        "Target.setAutoAttach" | "Target.setDiscoverTargets" | "Page.enable" => json!({}),

        "Page.getResourceTree" => {
            json!({
                "frameTree": {
                    "frame": {
                        "id": "1",
                        "loaderId": "1",
                        "url": "kismet://game",
                        "securityOrigin": "kismet://game",
                        "mimeType": "text/plain",
                    },
                    "resources": []
                }
            })
        }

        _ => {
            tracing::debug!("Unhandled CDP method: {method}");
            json!({})
        }
    };

    (result, events)
}

fn handle_dbg_event(event: &DbgEvent) -> Vec<Value> {
    match event {
        DbgEvent::Paused {
            script_idx,
            line,
            frame,
        } => {
            let state = debugger::shared();
            let stack = unsafe { &**frame };

            let call_frames: Vec<Value> = debugger::iter_frames(stack)
                .enumerate()
                .map(|(i, f)| {
                    let function_path = if !f.node().is_null() {
                        unsafe { (*(f.node() as *const ue::UObjectBase)).object_path() }
                    } else {
                        "???".into()
                    };
                    let object_path = if !f.object().is_null() {
                        unsafe { (*(f.object() as *const ue::UObjectBase)).object_path() }
                    } else {
                        "???".into()
                    };

                    let script_id = if !f.node().is_null() {
                        let node_weak = ue::TWeakObjectPtr::<ue::UFunction>::new(unsafe {
                            &*(f.node() as *const ue::UObjectBase)
                        });
                        let scripts = state.scripts.lock();
                        scripts
                            .iter()
                            .find(|s| s.function_for_weak(&node_weak).is_some())
                            .map(|s| s.script_id.clone())
                            .unwrap_or_else(|| "0".into())
                    } else {
                        "0".into()
                    };

                    json!({
                        "callFrameId": format!("{i}"),
                        "functionName": function_path,
                        "location": {
                            "scriptId": script_id,
                            "lineNumber": if i == 0 { *line } else { 0 },
                            "columnNumber": 0,
                        },
                        "url": format!("kismet://{function_path}"),
                        "scopeChain": [
                            {
                                "type": "local",
                                "object": {
                                    "type": "object",
                                    "objectId": format!("locals-{i}"),
                                    "description": "Local",
                                }
                            },
                            {
                                "type": "global",
                                "object": {
                                    "type": "object",
                                    "objectId": format!("this-{i}"),
                                    "description": object_path,
                                }
                            }
                        ],
                        "this": {
                            "type": "object",
                            "objectId": format!("this-{i}"),
                            "description": object_path,
                        }
                    })
                })
                .collect();

            vec![json!({
                "method": "Debugger.paused",
                "params": {
                    "callFrames": call_frames,
                    "reason": "other",
                    "hitBreakpoints": [],
                }
            })]
        }
        DbgEvent::ScriptParsed { script_idx } => {
            let state = debugger::shared();
            let scripts = state.scripts.lock();
            let Some(script) = scripts.get(*script_idx) else {
                return vec![];
            };
            vec![json!({
                "method": "Debugger.scriptParsed",
                "params": {
                    "scriptId": script.script_id,
                    "url": script.url,
                    "startLine": 0,
                    "startColumn": 0,
                    "endLine": script.line_count,
                    "endColumn": 0,
                    "executionContextId": 1,
                    "hash": format!("{:x}", script_idx),
                    "sourceMapURL": "",
                    "hasSourceURL": false,
                    "isModule": false,
                    "length": script.source.len(),
                }
            })]
        }
    }
}

/// Convert a byte offset within `source` to 0-indexed (line, column). Column
/// is counted in bytes, not Unicode code points — close enough for ASCII
/// Kismet source and matches DevTools' expectations for short error spans.
fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let clamped = offset.min(source.len());
    let prefix = &source.as_bytes()[..clamped];
    let line = prefix.iter().filter(|&&b| b == b'\n').count();
    let line_start = prefix
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let col = clamped - line_start;
    (line, col)
}

fn eval_repl(expr: &str) -> String {
    let expr = expr.trim();

    if let Some(inner) = expr.strip_prefix("addr(").and_then(|s| s.strip_suffix(')')) {
        let inner = inner.trim();
        let addr = if let Some(hex) = inner
            .strip_prefix("0x")
            .or_else(|| inner.strip_prefix("0X"))
        {
            u64::from_str_radix(hex, 16)
        } else {
            inner.parse::<u64>()
        };
        match addr {
            Ok(addr) => {
                let ptr = addr as *const ue::UObjectBase;
                if ptr.is_null() {
                    return "null".into();
                }
                let obj = unsafe { &*ptr };
                format!("{:#x} => {}", addr, obj.path())
            }
            Err(e) => format!("bad address: {e}"),
        }
    } else if let Some(inner) = expr.strip_prefix("path(").and_then(|s| s.strip_suffix(')')) {
        let path = inner.trim().trim_matches(|c| c == '\'' || c == '"');
        let ht = ue::hash_tables();
        match ht.find_object_by_path(path) {
            Some(obj) => {
                let addr = obj as *const ue::UObjectBase as u64;
                format!("{:#x} {}", addr, obj.path())
            }
            None => format!("not found: '{path}'"),
        }
    } else {
        format!(
            "unknown command: {expr}\nusage:\n  addr(0x1234)  — deref ptr, print object path\n  path('/Script/Engine')  — resolve path, print addr"
        )
    }
}
