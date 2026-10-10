use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RpcRequest {
    pub(crate) jsonrpc: String,
    pub(crate) id: Value,
    pub(crate) method: String,
    pub(crate) params: Value,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Configuration {
    #[serde(default)]
    pub(crate) accepted_output_modes: Vec<String>,
    pub(crate) task_push_notification_config: Option<Value>,
    pub(crate) history_length: Option<i32>,
    #[serde(default)]
    pub(crate) return_immediately: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SendRequest {
    #[serde(default)]
    pub(crate) tenant: String,
    pub(crate) message: Message,
    #[serde(default)]
    pub(crate) configuration: Configuration,
    pub(crate) metadata: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Message {
    pub(crate) message_id: String,
    pub(crate) context_id: Option<String>,
    pub(crate) task_id: Option<String>,
    pub(crate) role: String,
    pub(crate) parts: Vec<Value>,
    pub(crate) metadata: Option<Value>,
    #[serde(default)]
    pub(crate) extensions: Vec<String>,
    #[serde(default)]
    pub(crate) reference_task_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskRequest {
    #[serde(default)]
    pub(crate) tenant: String,
    pub(crate) id: String,
    pub(crate) history_length: Option<i32>,
    pub(crate) metadata: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListRequest {
    #[serde(default)]
    pub(crate) tenant: String,
    pub(crate) context_id: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) page_size: Option<i32>,
    pub(crate) page_token: Option<String>,
    pub(crate) history_length: Option<i32>,
    pub(crate) status_timestamp_after: Option<String>,
    #[serde(default)]
    pub(crate) include_artifacts: bool,
}

pub(crate) fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_:.".contains(&byte))
}
pub(crate) fn history_limit(value: Option<i32>) -> Result<u32, crate::http::RpcError> {
    match value {
        Some(value) if value < 0 => Err(crate::http::RpcError::invalid()),
        Some(value) => Ok(value.min(16) as u32),
        None => Ok(16),
    }
}
pub(crate) fn text(message: &Message) -> Result<String, crate::http::RpcError> {
    if !token(&message.message_id)
        || message.role != "ROLE_USER"
        || message.parts.is_empty()
        || message.parts.len() > 16
        || message
            .task_id
            .as_ref()
            .is_some_and(|id| !id.is_empty() && !token(id))
        || message
            .context_id
            .as_ref()
            .is_some_and(|id| !id.is_empty() && !token(id))
    {
        return Err(crate::http::RpcError::invalid());
    }
    if !message.extensions.is_empty() || !message.reference_task_ids.is_empty() {
        return Err(crate::http::RpcError::unsupported());
    }
    let mut text = String::new();
    for part in &message.parts {
        let Some(part) = part.as_object() else {
            return Err(crate::http::RpcError::invalid());
        };
        if part
            .keys()
            .any(|key| !matches!(key.as_str(), "text" | "metadata" | "mediaType" | "filename"))
        {
            return Err(crate::http::RpcError::media());
        }
        if part.get("mediaType").is_some_and(|value| {
            value
                .as_str()
                .is_none_or(|value| !matches!(value, "text/plain" | ""))
        }) || part
            .get("filename")
            .is_some_and(|value| value.as_str().is_none_or(|value| !value.is_empty()))
        {
            return Err(crate::http::RpcError::media());
        }
        let Some(content) = part.get("text").and_then(Value::as_str) else {
            return Err(crate::http::RpcError::media());
        };
        if text.len().saturating_add(content.len()) > 16 * 1024 {
            return Err(crate::http::RpcError::invalid());
        }
        text.push_str(content);
    }
    if text.is_empty() {
        return Err(crate::http::RpcError::invalid());
    }
    Ok(text)
}
