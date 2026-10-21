//! Explicit read-only scope classification. Arbitrary tool text never selects policy.
use super::*;
use colossus_contracts::{
    BrowserDocumentId, BrowserObservation, BrowserSessionId, BrowserTabId, ComputerLimits,
    ComputerObservation, ComputerObservationId, ComputerSessionId, ComputerWindowId,
    ComputerWindowSummary, ModelImageReference,
};

pub(super) fn native_read(name: &str) -> bool {
    matches!(
        name,
        "computer.windows"
            | "computer.snapshot"
            | "computer.screenshot"
            | "browser.snapshot"
            | "browser.screenshot"
            | "browser.screenshot.annotated"
    )
}

fn duplicate_read(name: &str) -> bool {
    matches!(
        name,
        "filesystem.list"
            | "filesystem.read"
            | "filesystem.search"
            | "git.status"
            | "git.diff"
            | "git.show"
            | "repo.map"
            | "repo.symbol_search"
            | "repo.references"
            | "repo.file_summary"
            | "tool.search"
    )
}

pub(super) fn records_outcome(name: &str) -> bool {
    native_read(name) || duplicate_read(name)
}

/// Capture only validated native identity before the ordinary byte reducer can shed nodes.
pub(super) fn released_identity(result: &ToolResult) -> Option<Value> {
    if result.exit_code != 0 || !native_read(&result.name) {
        return None;
    }
    let value = serde_json::from_str::<Value>(&result.output).ok()?;
    match result.name.as_str() {
        "computer.windows" => {
            only_keys(&value, &["windows"])?;
            let windows: Vec<ComputerWindowSummary> =
                serde_json::from_value(value.get("windows")?.clone()).ok()?;
            if windows.len() > usize::from(ComputerLimits::HARD_MAX.max_windows)
                || windows.iter().any(|window| window.validate().is_err())
            {
                return None;
            }
            Some(json!({"kind":"computer.windows"}))
        }
        "computer.snapshot" => {
            only_keys(&value, &["observation"])?;
            let observation: ComputerObservation =
                serde_json::from_value(value.get("observation")?.clone()).ok()?;
            observation.validate(&ComputerLimits::HARD_MAX).ok()?;
            Some(
                json!({"kind":"computer.snapshot", "session_id":observation.session_id,
                "control_generation":observation.control_generation, "window_id":observation.window_id,
                "observation_id":observation.observation_id}),
            )
        }
        "browser.snapshot" => {
            let observation: BrowserObservation = serde_json::from_value(value).ok()?;
            let snapshot = observation.snapshot.as_ref()?;
            if observation.inspection.is_some()
                || snapshot.nodes.len() > 1024
                || snapshot.document_id != observation.tab.document_id
                || snapshot.nodes.iter().any(|node| {
                    node.element.document_id != snapshot.document_id
                        || node.element.snapshot_id != snapshot.snapshot_id
                })
            {
                return None;
            }
            Some(
                json!({"kind":"browser.snapshot", "session_id":observation.session_id,
                "tab_id":observation.tab.tab_id, "document_id":observation.tab.document_id,
                "snapshot_id":snapshot.snapshot_id}),
            )
        }
        "computer.screenshot" | "browser.screenshot" | "browser.screenshot.annotated" => {
            only_keys(
                &value,
                if result.name == "computer.screenshot" {
                    &[
                        "session_id",
                        "control_generation",
                        "observation_id",
                        "window_id",
                        "artifact",
                    ]
                } else {
                    &["session_id", "control_generation", "target", "artifact"]
                },
            )?;
            released_tool_image_message(result)?;
            let artifact: ModelImageReference =
                serde_json::from_value(value.get("artifact")?.clone()).ok()?;
            if result.images.first() != Some(&artifact) {
                return None;
            }
            let generation = positive(&value, "control_generation")?;
            if result.name == "computer.screenshot" {
                let session: ComputerSessionId =
                    serde_json::from_value(value.get("session_id")?.clone()).ok()?;
                let window: ComputerWindowId =
                    serde_json::from_value(value.get("window_id")?.clone()).ok()?;
                let observation: ComputerObservationId =
                    serde_json::from_value(value.get("observation_id")?.clone()).ok()?;
                Some(
                    json!({"kind":result.name, "session_id":session, "control_generation":generation,
                    "window_id":window, "observation_id":observation}),
                )
            } else {
                let session: BrowserSessionId =
                    serde_json::from_value(value.get("session_id")?.clone()).ok()?;
                let target = value.get("target")?;
                let tab: BrowserTabId =
                    serde_json::from_value(target.get("tab_id")?.clone()).ok()?;
                let document: BrowserDocumentId =
                    serde_json::from_value(target.get("document_id")?.clone()).ok()?;
                Some(
                    json!({"kind":result.name, "session_id":session, "control_generation":generation,
                    "tab_id":tab, "document_id":document}),
                )
            }
        }
        _ => None,
    }
}

pub(super) fn observation_scope(
    call: &ModelToolCall,
    existing: ExistingObservation<'_>,
) -> Option<String> {
    if duplicate_read(&call.name) {
        // Exact repeated reads only. Changed files, query limits/filters, pagination,
        // and unique findings remain evidence even when their tool name matches.
        let arguments = canonical_arguments(&call.arguments, 16)?;
        return Some(
            json!([
                call.name,
                arguments,
                existing.original_bytes,
                existing.digest
            ])
            .to_string(),
        );
    }
    let identity = existing.metadata.identity?;
    if identity.get("kind")?.as_str()? != call.name {
        return None;
    }
    let arguments = call.arguments.as_object()?;
    let keys: &[&str] = match call.name.as_str() {
        "computer.windows" => &["max_windows"],
        "computer.snapshot" => &["session_id", "control_generation", "max_nodes"],
        "computer.screenshot" => &["session_id", "control_generation"],
        "browser.snapshot" => &[
            "session_id",
            "control_generation",
            "tab_id",
            "document_id",
            "max_nodes",
        ],
        "browser.screenshot" | "browser.screenshot.annotated" => {
            &["session_id", "control_generation", "tab_id", "document_id"]
        }
        _ => return None,
    };
    if arguments.keys().any(|key| !keys.contains(&key.as_str())) {
        return None;
    }
    if call.name == "computer.windows" {
        optional_limit(&call.arguments, "max_windows", 128)?;
        return Some(json!([call.name, call.arguments.get("max_windows")]).to_string());
    }
    let generation = positive(&call.arguments, "control_generation")?;
    let session = identity.get("session_id")?;
    if call.arguments.get("session_id")? != session {
        return None;
    }
    if call.name.starts_with("computer.") {
        let _: ComputerSessionId = serde_json::from_value(session.clone()).ok()?;
        let window: ComputerWindowId =
            serde_json::from_value(identity.get("window_id")?.clone()).ok()?;
        if identity.get("control_generation")?.as_u64()? != generation {
            return None;
        }
        optional_limit(&call.arguments, "max_nodes", 1024)?;
        Some(
            json!([
                call.name,
                session,
                window,
                generation,
                call.arguments.get("max_nodes")
            ])
            .to_string(),
        )
    } else {
        let _: BrowserSessionId = serde_json::from_value(session.clone()).ok()?;
        let tab: BrowserTabId = serde_json::from_value(identity.get("tab_id")?.clone()).ok()?;
        let _: BrowserDocumentId =
            serde_json::from_value(call.arguments.get("document_id")?.clone()).ok()?;
        if call.arguments.get("tab_id")? != identity.get("tab_id")? {
            return None;
        }
        if call.name == "browser.snapshot" {
            optional_limit(&call.arguments, "max_nodes", 1024)?;
            call.arguments.get("max_nodes")?;
        } else if identity.get("control_generation")?.as_u64()? != generation
            || identity.get("document_id")? != call.arguments.get("document_id")?
        {
            return None;
        }
        Some(
            json!([
                call.name,
                session,
                tab,
                generation,
                call.arguments.get("max_nodes")
            ])
            .to_string(),
        )
    }
}

fn positive(value: &Value, key: &str) -> Option<u64> {
    value.get(key)?.as_u64().filter(|value| *value > 0)
}

fn only_keys(value: &Value, keys: &[&str]) -> Option<()> {
    value
        .as_object()?
        .keys()
        .all(|key| keys.contains(&key.as_str()))
        .then_some(())
}

fn optional_limit(value: &Value, key: &str, maximum: u64) -> Option<()> {
    if let Some(value) = value.get(key) {
        (1..=maximum).contains(&value.as_u64()?).then_some(())
    } else {
        Some(())
    }
}

fn canonical_arguments(value: &Value, depth: usize) -> Option<Value> {
    if depth == 0 {
        return None;
    }
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| canonical_arguments(value, depth - 1))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| Some((key.clone(), canonical_arguments(value, depth - 1)?)))
            .collect::<Option<BTreeMap<_, _>>>()
            .map(|values| Value::Object(values.into_iter().collect())),
        _ => Some(value.clone()),
    }
}
