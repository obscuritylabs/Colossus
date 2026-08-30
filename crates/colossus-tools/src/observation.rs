use colossus_contracts::{ModelMessage, ModelMessageRole, ToolResult};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

/// Maximum serialized size of one model-visible tool-result message.
pub const MAX_MODEL_TOOL_MESSAGE_BYTES: usize = 64 * 1024;
/// Maximum combined serialized size of model-visible tool results in one user logical turn.
pub const MAX_MODEL_TOOL_TURN_BYTES: usize = 256 * 1024;

const MIN_TOOL_MESSAGE_BUDGET_BYTES: usize = 512;
pub(crate) const MAX_OBSERVATION_METADATA_TEXT_BYTES: usize = 128;
const MAX_JSON_DEPTH: usize = 16;
const MAX_BINARY_SANITIZE_DEPTH: usize = 128;
const IMPORTANT_KEYS: &[&str] = &[
    "id",
    "key",
    "name",
    "title",
    "summary",
    "status",
    "url",
    "type",
    "message",
    "error",
    "isError",
    "is_error",
    "server",
    "tool",
    "structuredContent",
    "structured_content",
    "content",
    "resource",
    "uri",
    "mediaType",
];

#[derive(Clone, Copy, Default)]
struct ObservationMetadata<'a> {
    tool_name: Option<&'a str>,
    call_id: Option<&'a str>,
    exit_code: Option<i32>,
}

#[derive(Clone, Copy)]
struct JsonReductionLimits {
    depth: usize,
    string_bytes: usize,
    array_items: usize,
    object_fields: usize,
}

#[derive(Clone, Copy)]
struct ExistingObservation<'a> {
    format: &'a str,
    original_bytes: u64,
    digest: &'a str,
    content_key: &'a str,
    content: &'a Value,
    metadata: ObservationMetadata<'a>,
}

/// Convert complete released tool results into bounded model-visible messages.
///
/// Complete results remain owned by the caller for audit, presentation, and any
/// tool-specific parsing. Only the returned provider-continuation messages are lossy.
pub fn tool_result_observation_messages(results: &[ToolResult]) -> Vec<ModelMessage> {
    let messages = results
        .iter()
        .map(|result| ModelMessage {
            role: ModelMessageRole::Tool,
            content: result.output.clone().into(),
            tool_call_id: Some(result.call_id.clone()),
            tool_calls: Vec::new(),
        })
        .collect::<Vec<_>>();
    let metadata = results
        .iter()
        .map(|result| ObservationMetadata {
            tool_name: Some(result.name.as_str()),
            call_id: Some(result.call_id.as_str()),
            exit_code: Some(result.exit_code),
        })
        .collect::<Vec<_>>();
    project_tool_messages(&messages, &metadata)
}

/// Return a provider-visible copy whose user logical turns obey model observation bounds.
///
/// This is also the compatibility path for sessions written before observations were
/// bounded. Non-tool messages remain byte-for-byte equivalent, tool messages remain
/// unchanged when their complete logical turn fits, and the canonical input slice is
/// never mutated.
pub fn project_model_tool_observations(messages: &[ModelMessage]) -> Vec<ModelMessage> {
    let mut projected = messages.to_vec();
    let mut logical_start = 0;
    for index in 1..=projected.len() {
        let logical_end =
            index == projected.len() || projected[index].role == ModelMessageRole::User;
        if !logical_end {
            continue;
        }
        project_logical_turn(&mut projected[logical_start..index]);
        logical_start = index;
    }
    projected
}

fn project_logical_turn(messages: &mut [ModelMessage]) {
    let tool_names = messages
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .map(|call| (call.call_id.as_str(), call.name.as_str()))
        .collect::<BTreeMap<_, _>>();
    let tool_indices = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| (message.role == ModelMessageRole::Tool).then_some(index))
        .collect::<Vec<_>>();
    let tool_messages = tool_indices
        .iter()
        .map(|index| messages[*index].clone())
        .collect::<Vec<_>>();
    let metadata = tool_messages
        .iter()
        .map(|message| ObservationMetadata {
            tool_name: message
                .tool_call_id
                .as_deref()
                .and_then(|call_id| tool_names.get(call_id).copied()),
            call_id: message.tool_call_id.as_deref(),
            exit_code: None,
        })
        .collect::<Vec<_>>();
    let projected = project_tool_messages(&tool_messages, &metadata);
    for (index, message) in tool_indices.into_iter().zip(projected) {
        messages[index] = message;
    }
}

fn project_tool_messages(
    messages: &[ModelMessage],
    metadata: &[ObservationMetadata<'_>],
) -> Vec<ModelMessage> {
    debug_assert_eq!(messages.len(), metadata.len());
    let serialized = messages
        .iter()
        .map(serialized_message_bytes)
        .collect::<Vec<_>>();
    let desired = serialized
        .iter()
        .map(|size| (*size).min(MAX_MODEL_TOOL_MESSAGE_BYTES))
        .collect::<Vec<_>>();
    let minimum = serialized
        .iter()
        .map(|size| (*size).min(MIN_TOOL_MESSAGE_BUDGET_BYTES))
        .collect::<Vec<_>>();
    let budgets = waterfill_budgets(&desired, &minimum, MAX_MODEL_TOOL_TURN_BYTES);

    messages
        .iter()
        .zip(metadata)
        .zip(budgets)
        .map(|((message, metadata), budget)| project_tool_message(message, *metadata, budget))
        .collect()
}

fn waterfill_budgets(desired: &[usize], minimum: &[usize], total: usize) -> Vec<usize> {
    if desired.iter().copied().sum::<usize>() <= total {
        return desired.to_vec();
    }
    let minimum_sum = minimum.iter().copied().sum::<usize>();
    if minimum_sum == total {
        return minimum.to_vec();
    }
    let minimum_is_feasible = minimum_sum < total;

    let mut low = 0_usize;
    let mut high = desired.iter().copied().max().unwrap_or_default();
    while low < high {
        let middle = low.saturating_add(high).saturating_add(1) / 2;
        let used = desired
            .iter()
            .zip(minimum)
            .map(|(desired, minimum)| {
                (*desired)
                    .min(middle)
                    .max(if minimum_is_feasible { *minimum } else { 0 })
            })
            .fold(0_usize, usize::saturating_add);
        if used <= total {
            low = middle;
        } else {
            high = middle.saturating_sub(1);
        }
    }

    let mut budgets = desired
        .iter()
        .zip(minimum)
        .map(|(desired, minimum)| {
            (*desired)
                .min(low)
                .max(if minimum_is_feasible { *minimum } else { 0 })
        })
        .collect::<Vec<_>>();
    let mut remaining = total.saturating_sub(budgets.iter().copied().sum::<usize>());
    for (budget, desired) in budgets.iter_mut().zip(desired) {
        if remaining == 0 {
            break;
        }
        let increment = desired.saturating_sub(*budget).min(remaining);
        *budget = budget.saturating_add(increment);
        remaining = remaining.saturating_sub(increment);
    }
    budgets
}

fn project_tool_message(
    message: &ModelMessage,
    metadata: ObservationMetadata<'_>,
    message_budget: usize,
) -> ModelMessage {
    if serialized_message_bytes(message) <= message_budget {
        return message.clone();
    }
    let output = message.content.plain_text();
    let output = output.as_ref();

    let parsed = serde_json::from_str::<Value>(output).ok();
    if let Some(existing) = parsed.as_ref().and_then(existing_observation) {
        return project_existing_tool_message(message, metadata, existing, message_budget);
    }
    let normalized = parsed.as_ref().map(normalize_oversized_json);
    let digest = sha256_hex(output.as_bytes());
    let mut output_budget = message_budget.saturating_sub(96).max(256);

    for _ in 0..16 {
        let observation = render_observation(
            output,
            normalized.as_ref(),
            metadata,
            &digest,
            output_budget,
        );
        let mut candidate = message.clone();
        candidate.content = observation.into();
        let candidate_bytes = serialized_message_bytes(&candidate);
        if candidate_bytes <= message_budget {
            return candidate;
        }
        let excess = candidate_bytes.saturating_sub(message_budget).max(1);
        let next = output_budget.saturating_sub(excess);
        if next >= output_budget || next < 256 {
            break;
        }
        output_budget = next;
    }

    let mut candidate = message.clone();
    candidate.content = metadata_only_observation(
        u64::try_from(output.len()).unwrap_or(u64::MAX),
        normalized.is_some(),
        metadata,
        &digest,
    )
    .to_string()
    .into();
    if serialized_message_bytes(&candidate) <= message_budget {
        return candidate;
    }

    // An infeasible preferred allocation can occur only for a malformed or legacy turn with
    // hundreds of results. Preserve provider correlation and shed observation content rather
    // than allowing the aggregate budget to grow without bound.
    candidate.content = String::new().into();
    debug_assert!(
        serialized_message_bytes(&candidate) <= message_budget,
        "the structural tool-result message exceeds its aggregate allocation"
    );
    candidate
}

fn existing_observation(value: &Value) -> Option<ExistingObservation<'_>> {
    let details = value.get("_colossusToolObservation")?.as_object()?;
    if !details.get("truncated")?.as_bool()? {
        return None;
    }
    let format = details.get("format")?.as_str()?;
    if !matches!(format, "json" | "text") {
        return None;
    }
    let original_bytes = details.get("originalBytes")?.as_u64()?;
    let digest = details.get("sha256")?.as_str()?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let (content_key, content) = if let Some(content) = value.get("preview") {
        ("preview", content)
    } else {
        ("data", value.get("data")?)
    };
    Some(ExistingObservation {
        format,
        original_bytes,
        digest,
        content_key,
        content,
        metadata: ObservationMetadata {
            tool_name: details.get("toolName").and_then(Value::as_str),
            call_id: details.get("callId").and_then(Value::as_str),
            exit_code: details
                .get("exitCode")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok()),
        },
    })
}

fn project_existing_tool_message(
    message: &ModelMessage,
    metadata: ObservationMetadata<'_>,
    existing: ExistingObservation<'_>,
    message_budget: usize,
) -> ModelMessage {
    let metadata = ObservationMetadata {
        tool_name: metadata.tool_name.or(existing.metadata.tool_name),
        call_id: metadata.call_id.or(existing.metadata.call_id),
        exit_code: metadata.exit_code.or(existing.metadata.exit_code),
    };
    let mut output_budget = message_budget.saturating_sub(96).max(256);
    for _ in 0..16 {
        let observation = render_existing_observation(existing, metadata, output_budget);
        let mut candidate = message.clone();
        candidate.content = observation.into();
        let candidate_bytes = serialized_message_bytes(&candidate);
        if candidate_bytes <= message_budget {
            return candidate;
        }
        let excess = candidate_bytes.saturating_sub(message_budget).max(1);
        let next = output_budget.saturating_sub(excess);
        if next >= output_budget || next < 256 {
            break;
        }
        output_budget = next;
    }

    let mut candidate = message.clone();
    candidate.content = metadata_only_observation(
        existing.original_bytes,
        existing.format == "json",
        metadata,
        existing.digest,
    )
    .to_string()
    .into();
    if serialized_message_bytes(&candidate) <= message_budget {
        return candidate;
    }
    candidate.content = String::new().into();
    debug_assert!(
        serialized_message_bytes(&candidate) <= message_budget,
        "the structural tool-result message exceeds its aggregate allocation"
    );
    candidate
}

fn render_existing_observation(
    existing: ExistingObservation<'_>,
    metadata: ObservationMetadata<'_>,
    budget: usize,
) -> String {
    match (existing.format, existing.content_key) {
        ("json", "data") => render_json_observation(
            existing.original_bytes,
            existing.content,
            metadata,
            existing.digest,
            budget,
        ),
        ("text", "preview") => existing.content.as_str().map_or_else(
            || {
                metadata_only_observation(existing.original_bytes, false, metadata, existing.digest)
                    .to_string()
            },
            |preview| {
                render_text_observation(
                    preview,
                    existing.original_bytes,
                    metadata,
                    existing.digest,
                    budget,
                )
            },
        ),
        _ => metadata_only_observation(
            existing.original_bytes,
            existing.format == "json",
            metadata,
            existing.digest,
        )
        .to_string(),
    }
}

fn render_observation(
    output: &str,
    normalized: Option<&Value>,
    metadata: ObservationMetadata<'_>,
    digest: &str,
    budget: usize,
) -> String {
    let original_bytes = u64::try_from(output.len()).unwrap_or(u64::MAX);
    if let Some(value) = normalized {
        render_json_observation(original_bytes, value, metadata, digest, budget)
    } else {
        render_text_observation(output, original_bytes, metadata, digest, budget)
    }
}

fn render_json_observation(
    original_bytes: u64,
    value: &Value,
    metadata: ObservationMetadata<'_>,
    digest: &str,
    budget: usize,
) -> String {
    let value = omit_binary_values(value, MAX_BINARY_SANITIZE_DEPTH, None);
    let full = observation_envelope(
        original_bytes,
        "json",
        metadata,
        digest,
        "data",
        value.clone(),
    );
    if serialized_value_bytes(&full) <= budget {
        return full.to_string();
    }

    for step in 0_u32..8 {
        let limits = JsonReductionLimits {
            depth: (MAX_JSON_DEPTH >> step).max(1),
            string_bytes: ((8 * 1024_usize) >> step).max(48),
            array_items: (32_usize >> step).max(2),
            object_fields: (64_usize >> step).max(2),
        };
        let reduced = reduce_json(&value, limits.depth, limits, None);
        let candidate =
            observation_envelope(original_bytes, "json", metadata, digest, "data", reduced);
        if serialized_value_bytes(&candidate) <= budget {
            return candidate.to_string();
        }
    }

    metadata_only_observation(original_bytes, true, metadata, digest).to_string()
}

fn omit_binary_values(value: &Value, remaining_depth: usize, key: Option<&str>) -> Value {
    if remaining_depth == 0 {
        return json!({"_colossusTruncatedDepth": true});
    }
    match value {
        Value::String(text) if binary_value(key, text) => json!({
            "_colossusBinaryOmitted": {
                "encodedBytes": text.len(),
                "sha256": sha256_hex(text.as_bytes()),
            }
        }),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| omit_binary_values(value, remaining_depth - 1, None))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        omit_binary_values(value, remaining_depth - 1, Some(key)),
                    )
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn render_text_observation(
    output: &str,
    original_bytes: u64,
    metadata: ObservationMetadata<'_>,
    digest: &str,
    budget: usize,
) -> String {
    let mut preview_budget = budget.saturating_sub(256).min(output.len());
    loop {
        let preview = head_tail_text(output, preview_budget);
        let candidate = observation_envelope(
            original_bytes,
            "text",
            metadata,
            digest,
            "preview",
            Value::String(preview),
        );
        let bytes = serialized_value_bytes(&candidate);
        if bytes <= budget {
            return candidate.to_string();
        }
        let next = preview_budget.saturating_sub(bytes.saturating_sub(budget).max(1));
        if next >= preview_budget || next == 0 {
            return metadata_only_observation(original_bytes, false, metadata, digest).to_string();
        }
        preview_budget = next;
    }
}

fn observation_envelope(
    original_bytes: u64,
    format: &str,
    metadata: ObservationMetadata<'_>,
    digest: &str,
    content_key: &str,
    content: Value,
) -> Value {
    let mut details = Map::new();
    details.insert("truncated".into(), Value::Bool(true));
    details.insert("format".into(), Value::String(format.into()));
    if let Some(tool_name) = metadata.tool_name {
        details.insert(
            "toolName".into(),
            Value::String(head_tail_text(
                tool_name,
                MAX_OBSERVATION_METADATA_TEXT_BYTES,
            )),
        );
    }
    if let Some(call_id) = metadata.call_id {
        details.insert(
            "callId".into(),
            Value::String(head_tail_text(call_id, MAX_OBSERVATION_METADATA_TEXT_BYTES)),
        );
    }
    if let Some(exit_code) = metadata.exit_code {
        details.insert("exitCode".into(), Value::from(exit_code));
    }
    details.insert("originalBytes".into(), Value::from(original_bytes));
    details.insert("sha256".into(), Value::String(digest.into()));

    let mut envelope = Map::new();
    envelope.insert("_colossusToolObservation".into(), Value::Object(details));
    envelope.insert(content_key.into(), content);
    Value::Object(envelope)
}

fn metadata_only_observation(
    original_bytes: u64,
    json_format: bool,
    metadata: ObservationMetadata<'_>,
    digest: &str,
) -> Value {
    observation_envelope(
        original_bytes,
        if json_format { "json" } else { "text" },
        metadata,
        digest,
        "data",
        json!({"_colossusTruncated": true}),
    )
}

fn normalize_oversized_json(value: &Value) -> Value {
    let Some(root) = value.as_object() else {
        return value.clone();
    };
    let Some(result) = root.get("result").and_then(Value::as_object) else {
        return value.clone();
    };
    if !root.contains_key("server") || !root.contains_key("tool") {
        return value.clone();
    }

    let mut normalized_result = result.clone();
    let structured_key = ["structuredContent", "structured_content"]
        .into_iter()
        .find(|key| result.get(*key).is_some_and(|value| !value.is_null()));
    if let Some(structured_key) = structured_key {
        let structured = result.get(structured_key).expect("key was resolved");
        if let Some(content) = result.get("content").and_then(Value::as_array) {
            let retained = content
                .iter()
                .filter(|block| !text_block_matches_json(block, structured))
                .cloned()
                .collect::<Vec<_>>();
            normalized_result.insert("content".into(), Value::Array(retained));
        }
    } else if let Some(content) = result.get("content").and_then(Value::as_array) {
        let parsed_text = content.iter().find_map(parse_text_block_json);
        if let Some(parsed) = parsed_text {
            normalized_result.insert("structuredContent".into(), parsed.clone());
            let retained = content
                .iter()
                .filter(|block| parse_text_block_json(block).as_ref() != Some(&parsed))
                .cloned()
                .collect::<Vec<_>>();
            normalized_result.insert("content".into(), Value::Array(retained));
        }
    }

    let mut normalized = root.clone();
    normalized.insert("result".into(), Value::Object(normalized_result));
    Value::Object(normalized)
}

fn text_block_matches_json(block: &Value, expected: &Value) -> bool {
    parse_text_block_json(block).as_ref() == Some(expected)
}

fn parse_text_block_json(block: &Value) -> Option<Value> {
    let object = block.as_object()?;
    if object.get("type")?.as_str()? != "text" {
        return None;
    }
    serde_json::from_str(object.get("text")?.as_str()?).ok()
}

fn reduce_json(
    value: &Value,
    remaining_depth: usize,
    limits: JsonReductionLimits,
    key: Option<&str>,
) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) => {
            if binary_value(key, text) {
                json!({
                    "_colossusBinaryOmitted": {
                        "encodedBytes": text.len(),
                        "sha256": sha256_hex(text.as_bytes()),
                    }
                })
            } else {
                Value::String(head_tail_text(text, limits.string_bytes))
            }
        }
        Value::Array(values) => {
            if remaining_depth == 0 {
                return json!({"_colossusTruncatedArray": {"items": values.len()}});
            }
            let keep = limits.array_items.min(values.len());
            if keep == values.len() {
                return Value::Array(
                    values
                        .iter()
                        .map(|value| reduce_json(value, remaining_depth - 1, limits, None))
                        .collect(),
                );
            }
            let head = keep.div_ceil(2);
            let tail = keep / 2;
            let mut reduced = values[..head]
                .iter()
                .map(|value| reduce_json(value, remaining_depth - 1, limits, None))
                .collect::<Vec<_>>();
            reduced.push(json!({
                "_colossusOmittedItems": values.len().saturating_sub(keep)
            }));
            reduced.extend(
                values[values.len().saturating_sub(tail)..]
                    .iter()
                    .map(|value| reduce_json(value, remaining_depth - 1, limits, None)),
            );
            Value::Array(reduced)
        }
        Value::Object(object) => {
            if remaining_depth == 0 {
                return json!({"_colossusTruncatedObject": {"fields": object.len()}});
            }
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_by(|left, right| {
                important_key_rank(left)
                    .cmp(&important_key_rank(right))
                    .then_with(|| {
                        json_value_rank(&object[*left]).cmp(&json_value_rank(&object[*right]))
                    })
                    .then_with(|| left.cmp(right))
            });
            let retained = limits.object_fields.min(keys.len());
            let mut reduced = Map::new();
            for key in keys.into_iter().take(retained) {
                reduced.insert(
                    key.clone(),
                    reduce_json(&object[key], remaining_depth - 1, limits, Some(key)),
                );
            }
            if retained < object.len() {
                reduced.insert(
                    "_colossusOmittedFields".into(),
                    Value::from(u64::try_from(object.len() - retained).unwrap_or(u64::MAX)),
                );
            }
            Value::Object(reduced)
        }
    }
}

fn important_key_rank(key: &str) -> usize {
    IMPORTANT_KEYS
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(key))
        .unwrap_or(IMPORTANT_KEYS.len())
}

const fn json_value_rank(value: &Value) -> u8 {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 0,
        Value::Array(_) | Value::Object(_) => 1,
    }
}

fn binary_value(key: Option<&str>, value: &str) -> bool {
    if value.len() < 4 * 1024 {
        return false;
    }
    let likely_key = key.is_some_and(|key| {
        matches!(
            key.to_ascii_lowercase().as_str(),
            "data" | "blob" | "bytes" | "image" | "audio"
        )
    });
    likely_key
        && value.len().is_multiple_of(4)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_' | b'\r' | b'\n')
        })
}

fn head_tail_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    if limit < 32 {
        return prefix_at_boundary(value, limit).into();
    }
    let marker = format!("…{} bytes omitted…", value.len().saturating_sub(limit));
    let content_budget = limit.saturating_sub(marker.len());
    let head_budget = content_budget.saturating_mul(3) / 5;
    let tail_budget = content_budget.saturating_sub(head_budget);
    let head = prefix_at_boundary(value, head_budget);
    let tail = suffix_at_boundary(value, tail_budget);
    format!("{head}{marker}{tail}")
}

fn prefix_at_boundary(value: &str, limit: usize) -> &str {
    let mut end = limit.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &value[..end]
}

fn suffix_at_boundary(value: &str, limit: usize) -> &str {
    let mut start = value.len().saturating_sub(limit);
    while start < value.len() && !value.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    &value[start..]
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn serialized_message_bytes(message: &ModelMessage) -> usize {
    serde_json::to_vec(message).map_or(usize::MAX, |bytes| bytes.len())
}

fn serialized_value_bytes(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}
