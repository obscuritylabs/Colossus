//! Decode the real model-facing receipt without confusing its envelope with page state.
use super::{TOOLS, alias, categorical_browser_failure, categorical_tool_failure};
use anyhow::{Context as _, ensure};
use serde_json::Value;

pub(super) fn tool_result(body: &Value, call_id: &str) -> anyhow::Result<Value> {
    let messages = body["messages"]
        .as_array()
        .context("model messages missing")?;
    let results = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message["role"] == "tool" && message["tool_call_id"] == call_id)
        .collect::<Vec<_>>();
    ensure!(
        results.len() == 1,
        "Core tool evidence correlation is not unique"
    );
    let (index, message) = results[0];
    let calls = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message["role"] == "assistant")
        .flat_map(|(position, message)| {
            message["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |call| (position, call))
        })
        .filter(|(_, call)| call["id"] == call_id)
        .collect::<Vec<_>>();
    ensure!(
        calls.len() == 1 && calls[0].0 < index,
        "Core tool evidence lacks one preceding assistant call"
    );
    let call = calls[0].1;
    ensure!(
        call["type"] == "function",
        "Core tool call is not a function"
    );
    let wire_name = call["function"]["name"]
        .as_str()
        .context("Core tool call name is missing")?;
    let name = TOOLS
        .iter()
        .copied()
        .find(|name| *name == wire_name || name.replace('.', "_") == wire_name)
        .context("Core tool evidence names an unowned fixture tool")?;
    ensure!(
        alias(body, name)? == wire_name,
        "Core tool call differs from its offered name"
    );
    let content = message["content"]
        .as_str()
        .context("tool evidence is not scalar text")?;
    let result: Value = serde_json::from_str(content).context("tool evidence is not JSON")?;
    decode(result, name, call_id)
}

fn failure(result: &Value) -> Option<&'static str> {
    if result.get("error").is_none_or(Value::is_null)
        && result.get("isError") != Some(&Value::Bool(true))
        && result.get("is_error") != Some(&Value::Bool(true))
    {
        return None;
    }
    let code = categorical_tool_failure(result);
    eprintln!(
        "EVIDENCE core-tool-failure code={code} browser_error={}",
        categorical_browser_failure(result)
    );
    Some(code)
}

fn decode(result: Value, name: &str, call_id: &str) -> anyhow::Result<Value> {
    let Some(metadata) = result.get("_colossusToolObservation") else {
        if let Some(category) = failure(&result) {
            anyhow::bail!("Core tool failed: {category}");
        }
        // Actions/status keep their original scalar receipts. Current native read
        // tools always issue trusted outcome metadata, even for a small result.
        ensure!(
            !matches!(name, "browser.snapshot" | "browser.screenshot"),
            "Core native observation lacks its outcome envelope"
        );
        ensure!(result.is_object(), "Core tool receipt is not an object");
        return Ok(result);
    };
    ensure!(
        metadata.is_object(),
        "Core observation metadata is malformed"
    );
    ensure!(
        metadata["toolName"] == name && metadata["callId"] == call_id,
        "Core observation metadata does not match its assistant call"
    );
    let data = result
        .get("data")
        .context("Core observation data is missing")?;
    let error = failure(data);
    ensure!(
        metadata["exitCode"].as_i64() == Some(0) && error.is_none(),
        "Core observation did not report a successful outcome"
    );
    ensure!(
        metadata["truncated"] == false
            && metadata["format"] == "json"
            && metadata["originalBytes"]
                .as_u64()
                .is_some_and(|bytes| bytes > 0)
            && metadata["sha256"].as_str().is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            && result.as_object().is_some_and(|object| object.len() == 2)
            && data.is_object(),
        "Core observation is incomplete or malformed"
    );
    Ok(data.clone())
}

#[cfg(test)]
pub(super) fn test_body(name: &str, call_id: &str, output: &Value) -> Value {
    let messages = if call_id.is_empty() {
        Vec::new()
    } else {
        let result = colossus_contracts::ToolResult {
            name: name.into(),
            call_id: call_id.into(),
            output: output.to_string(),
            exit_code: 0,
            images: Vec::new(),
        };
        let projected = colossus_tools::tool_result_observation_messages(&[result]);
        vec![
            serde_json::json!({"role":"assistant","tool_calls":[{"id":call_id,"type":"function",
                "function":{"name":name.replace('.',"_"),"arguments":"{}"}}]}),
            serde_json::json!({"role":"tool","tool_call_id":call_id,"content":projected[0].content.plain_text()}),
        ]
    };
    serde_json::json!({"tools":TOOLS.iter().map(|name|
        serde_json::json!({"function":{"name":name.replace('.',"_")}})).collect::<Vec<_>>(),
        "messages":messages})
}

#[cfg(test)]
mod tests;
