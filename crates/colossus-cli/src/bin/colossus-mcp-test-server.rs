//! Deterministic stdio MCP fixture used only by the workspace acceptance suite.

use serde_json::{Value, json};
use std::{
    io::{self, BufRead as _, Write as _},
    sync::mpsc,
    thread,
    time::Duration,
};

enum InputEvent {
    Request(Value),
    Eof,
    Invalid(String),
}

fn write_message(value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn handle_request(request: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Ok(());
    };
    let id = request.get("id").cloned();
    match method {
        "initialize" => write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "colossus-mcp-test-server", "version": "1.0.0"}
            }
        }))?,
        "notifications/initialized" => {}
        "tools/list" => {
            let cursor = request.pointer("/params/cursor").and_then(Value::as_str);
            let result = if cursor == Some("page-2") {
                json!({
                    "tools": [{
                        "name": "secret",
                        "description": "Return a nested fixture result.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false
                        }
                    }]
                })
            } else {
                json!({
                    "tools": [
                        {
                            "name": "echo",
                            "title": "Fixture echo",
                            "description": "Echo one text value.",
                            "annotations": {
                                "title": "Fixture echo",
                                "readOnlyHint": true,
                                "destructiveHint": false,
                                "idempotentHint": true,
                                "openWorldHint": false
                            },
                            "inputSchema": {
                                "type": "object",
                                "properties": {"text": {"type": "string"}},
                                "required": ["text"],
                                "additionalProperties": false
                            }
                        },
                        {
                            "name": "blocked",
                            "description": "This tool must be removed by the client allowlist.",
                            "inputSchema": {"type": "object"}
                        }
                    ],
                    "nextCursor": "page-2"
                })
            };
            write_message(&json!({"jsonrpc": "2.0", "id": id, "result": result}))?;
        }
        "tools/call" => {
            let tool = request.pointer("/params/name").and_then(Value::as_str);
            let arguments = request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let secret = std::env::var("MCP_TEST_SECRET").unwrap_or_default();
            let result = match tool {
                Some("echo") => json!({
                    "content": [{"type": "text", "text": format!("echo={arguments}; secret={secret}")}],
                    "structuredContent": {"arguments": arguments, "credential": secret},
                    "isError": false
                }),
                Some("secret") => json!({
                    "content": [{"type": "text", "text": secret}],
                    "isError": false
                }),
                _ => {
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": "unknown fixture tool"}
                    }))?;
                    return Ok(());
                }
            };
            write_message(&json!({"jsonrpc": "2.0", "id": id, "result": result}))?;
        }
        _ => write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": "method not found"}
        }))?,
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Keep enough idle tasks alive to catch Linux accounting regressions where
    // sysinfo task entries are mistaken for processes and their RSS is summed repeatedly.
    for _ in 0..32 {
        thread::spawn(thread::park);
    }

    // FastMCP-style dispatch reads stdin independently from operation handling. An
    // immediate EOF cancels a queued operation; keeping stdin open lets it complete.
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let event = match line {
                Ok(line) => match serde_json::from_str(&line) {
                    Ok(request) => InputEvent::Request(request),
                    Err(error) => InputEvent::Invalid(error.to_string()),
                },
                Err(error) => InputEvent::Invalid(error.to_string()),
            };
            if sender.send(event).is_err() {
                return;
            }
        }
        let _ = sender.send(InputEvent::Eof);
    });

    loop {
        let event = receiver.recv()?;
        let InputEvent::Request(request) = event else {
            return match event {
                InputEvent::Eof => Ok(()),
                InputEvent::Invalid(error) => Err(error.into()),
                InputEvent::Request(_) => unreachable!(),
            };
        };
        let operation = matches!(
            request.get("method").and_then(Value::as_str),
            Some("tools/list" | "tools/call")
        );
        if operation {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Ok(InputEvent::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Ok(InputEvent::Invalid(error)) => return Err(error.into()),
                Ok(InputEvent::Request(_)) => {
                    return Err("fixture received a request after its one-shot operation".into());
                }
            }
        }
        handle_request(&request)?;
    }
}
