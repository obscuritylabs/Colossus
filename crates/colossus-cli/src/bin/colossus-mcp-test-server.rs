//! Deterministic stdio MCP fixture used only by the workspace acceptance suite.

use serde_json::{Value, json};
use std::io::{self, BufRead as _, Write as _};

fn write_message(value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            continue;
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
                        continue;
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
    }
    Ok(())
}
