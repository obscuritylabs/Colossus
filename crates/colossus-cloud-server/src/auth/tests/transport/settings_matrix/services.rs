//! Loopback-only effects for the inherited-settings onboarding acceptance matrix.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct Observation {
    pub path: String,
    pub authorization: Option<String>,
    pub method: String,
    pub payload: Option<Value>,
}

pub(super) struct Services {
    pub origin: String,
    observations: Arc<Mutex<Vec<Observation>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Services {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let observations = Arc::new(Mutex::new(Vec::new()));
        let routes = Router::new()
            .route("/v1/models", get(models))
            .route("/v1/chat/completions", post(completion))
            .route("/mcp", post(mcp))
            .route("/search", get(search))
            .layer(DefaultBodyLimit::max(1024 * 1024))
            .with_state(observations.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, routes).await.unwrap();
        });
        Self {
            origin,
            observations,
            task,
        }
    }

    pub fn observations(&self) -> Vec<Observation> {
        self.observations.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.observations.lock().unwrap().clear();
    }

    pub async fn stop(self) {
        self.task.abort();
        assert!(self.task.await.unwrap_err().is_cancelled());
    }
}

type Observations = Arc<Mutex<Vec<Observation>>>;

fn record(
    observations: &Observations,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    payload: Option<Value>,
) -> Result<(), StatusCode> {
    let authorization = headers
        .get("authorization")
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let accepted = authorization
        .as_ref()
        .is_none_or(|value| value.starts_with("Bearer matrix-"));
    let mut records = observations.lock().unwrap();
    if records.len() >= 1024 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    records.push(Observation {
        path: path.into(),
        authorization,
        method: method.into(),
        payload,
    });
    accepted.then_some(()).ok_or(StatusCode::UNAUTHORIZED)
}

async fn models(State(observations): State<Observations>, headers: HeaderMap) -> Response {
    if let Err(status) = record(&observations, &headers, "GET", "/v1/models", None) {
        return status.into_response();
    }
    Json(json!({"object":"list","data":[{"id":"matrix-model","object":"model"}]})).into_response()
}

async fn completion(
    State(observations): State<Observations>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    if let Err(status) = record(
        &observations,
        &headers,
        "POST",
        "/v1/chat/completions",
        Some(input.clone()),
    ) {
        return status.into_response();
    }
    let Some(messages) = input["messages"].as_array() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let current = messages
        .iter()
        .rposition(|message| message["role"] == "user")
        .map_or(messages.as_slice(), |index| &messages[index..]);
    let prompt = current
        .first()
        .map(|message| message["content"].to_string())
        .unwrap_or_default();
    let called = current
        .iter()
        .filter_map(|message| message["tool_calls"].as_array())
        .flatten()
        .filter_map(|call| call["function"]["name"].as_str())
        .collect::<Vec<_>>();
    let tool_outputs = current
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| message["content"].as_str())
        .collect::<Vec<_>>();
    let action = if prompt.contains("[matrix:mcp]") {
        if !called.contains(&"mcp_servers") {
            Some(("mcp_servers", json!({})))
        } else {
            let server = tool_outputs.iter().find_map(|output| {
                serde_json::from_str(output)
                    .ok()
                    .and_then(|value| server_name(&value, 0))
            });
            let Some(server) = server else {
                return StatusCode::BAD_GATEWAY.into_response();
            };
            if !called.contains(&"mcp_tools") {
                Some(("mcp_tools", json!({"server":server,"tool":"ping"})))
            } else if !called.contains(&"mcp_call") {
                Some((
                    "mcp_call",
                    json!({"server":server,"tool":"ping","arguments":{}}),
                ))
            } else {
                None
            }
        }
    } else if prompt.contains("[matrix:search]") && !called.contains(&"web_search") {
        Some(("web_search", json!({"query":"matrix evidence","limit":1})))
    } else {
        None
    };
    let message = if let Some((name, arguments)) = action {
        // Consume the actual portable function names advertised by the runtime.
        let function = input["tools"].as_array().and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["function"]["name"] == name)
                .and_then(|tool| tool["function"]["name"].as_str())
        });
        let Some(function) = function else {
            return StatusCode::BAD_GATEWAY.into_response();
        };
        json!({"role":"assistant","content":null,"tool_calls":[{
            "id":format!("matrix-{function}"),"type":"function",
            "function":{"name":function,"arguments":arguments.to_string()}
        }]})
    } else {
        let success = if prompt.contains("[matrix:mcp]") {
            tool_outputs
                .iter()
                .any(|output| output.contains("matrix-mcp-result"))
        } else if prompt.contains("[matrix:search]") {
            tool_outputs
                .iter()
                .any(|output| output.contains("local matrix evidence"))
        } else {
            true
        };
        json!({"role":"assistant","content":if success {"matrix-complete"} else {"matrix-tool-failed"}})
    };
    completion_response(&input, message)
}

fn completion_response(input: &Value, message: Value) -> Response {
    let finish = if message.get("tool_calls").is_some() {
        "tool_calls"
    } else {
        "stop"
    };
    let usage = json!({"prompt_tokens":16,"completion_tokens":8,"total_tokens":24});
    if input["stream"] == true {
        let mut delta = message;
        if let Some(calls) = delta["tool_calls"].as_array_mut() {
            for (index, call) in calls.iter_mut().enumerate() {
                call["index"] = json!(index);
            }
        }
        let chunk = json!({"id":"matrix-completion","object":"chat.completion.chunk",
            "model":"matrix-model","choices":[{"index":0,"delta":delta,"finish_reason":finish}],"usage":usage});
        (
            [("content-type", "text/event-stream")],
            format!("data: {chunk}\n\ndata: [DONE]\n\n"),
        )
            .into_response()
    } else {
        Json(
            json!({"id":"matrix-completion","object":"chat.completion","model":"matrix-model",
            "choices":[{"index":0,"message":message,"finish_reason":finish}],"usage":usage}),
        )
        .into_response()
    }
}

fn server_name(value: &Value, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    if value.get("transport").and_then(Value::as_str) == Some("streamable_http")
        && value.get("available").and_then(Value::as_bool) != Some(false)
        && let Some(name) = value.get("name").and_then(Value::as_str)
    {
        return Some(name.into());
    }
    match value {
        Value::Array(values) => values
            .iter()
            .find_map(|value| server_name(value, depth + 1)),
        Value::Object(values) => values
            .values()
            .find_map(|value| server_name(value, depth + 1)),
        Value::String(text) => serde_json::from_str(text)
            .ok()
            .and_then(|value| server_name(&value, depth + 1)),
        _ => None,
    }
}

async fn mcp(
    State(observations): State<Observations>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    if let Err(status) = record(&observations, &headers, "POST", "/mcp", Some(input.clone())) {
        return status.into_response();
    }
    let result = match input["method"].as_str() {
        Some("notifications/initialized") => return StatusCode::ACCEPTED.into_response(),
        Some("server/discover") => {
            json!({"resultType":"complete","supportedVersions":["2026-07-28"],
            "capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"})
        }
        Some("initialize") => json!({"protocolVersion":"2026-07-28","capabilities":{"tools":{}},
            "serverInfo":{"name":"matrix","version":"1"}}),
        Some("tools/list") => {
            json!({"tools":[{"name":"ping","description":"Return local fixture evidence",
            "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}}]})
        }
        Some("tools/call") if input["params"]["name"] == "ping" => {
            json!({"content":[{"type":"text","text":"matrix-mcp-result"}],"isError":false})
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    Json(json!({"jsonrpc":"2.0","id":input["id"],"result":result})).into_response()
}

async fn search(State(observations): State<Observations>, headers: HeaderMap) -> Response {
    if let Err(status) = record(&observations, &headers, "GET", "/search", None) {
        return status.into_response();
    }
    Json(
        json!({"query":"matrix evidence","results":[{"url":"https://evidence.example/doc",
        "title":"Matrix evidence","content":"local matrix evidence"}]}),
    )
    .into_response()
}
