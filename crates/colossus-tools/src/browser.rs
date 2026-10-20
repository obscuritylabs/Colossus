use colossus_contracts::ToolSpec;
use serde_json::{Value, json};

#[path = "browser_interactions.rs"]
mod interactions;

/// First-party browser tools retain distinct effect identities. Availability is
/// resolved by trusted host composition; listing these schemas grants no access.
pub(super) fn browser_specs() -> Vec<ToolSpec> {
    let mut specs = vec![
        tool(
            "browser.open",
            "Open an isolated owned browser session. A supported Desktop engine resumes this conversation's suspended page when the allocation options match, using a new run lease; take a fresh snapshot before input. The original network deadline never extends across turns: an expired retained page is closed and a new session opens. Workspace settings and policy govern public HTTPS browsing and page dependencies; otherwise every request is restricted to the listed origins. Use embedded presentation in Desktop to let the user watch, or headless where supported.",
            object_schema(
                json!({
                    "mode": {"type": "string", "enum": ["embedded", "headless"]},
                    "allowed_origins": {
                        "type": "array", "minItems": 1, "maxItems": 32, "uniqueItems": true,
                        "items": {
                            "type": "string", "minLength": 1, "maxLength": 4096,
                            "pattern": "^https?://[^/@?#\\s]+/?$"
                        },
                        "description": "Initial task origins, without credentials, paths, queries, or fragments. These do not grant network access. Private destinations require exact policy admission; public HTTPS scope is chosen by the workspace, never tool arguments."
                    },
                    "initial_url": url_schema(),
                    "profile": {"oneOf": [
                        object_schema(json!({"kind":{"const":"temporary"}}), &["kind"]),
                        object_schema(json!({"kind":{"const":"workspace"},"id":id_schema("bp")}), &["kind", "id"])
                    ], "description":"Temporary by default. An existing explicit workspace profile is checked against authenticated native ownership; its opaque ID grants no access."}
                }),
                &["mode", "allowed_origins"],
            ),
        ),
        tool(
            "browser.status",
            "Read bounded status for one owned browser session. Omit session_id to discover only this active run's already granted sessions, including a human page handed to the agent. This never grants control or reveals credentials or engine endpoints.",
            object_schema(json!({"session_id": id_schema("bs")}), &[]),
        ),
        tool(
            "browser.tabs",
            "List bounded metadata for tabs in one owned browser session.",
            object_schema(json!({"session_id": id_schema("bs")}), &["session_id"]),
        ),
        tool(
            "browser.tab.open",
            "Create an owned tab under the current control generation, optionally navigating within the session's authorized network scope.",
            control_schema(json!({"url": url_schema()}), &[]),
        ),
        tool(
            "browser.tab.select",
            "Select an owned tab under the current control generation. Selecting a tab changes session presentation and control state.",
            control_schema(json!({"tab_id": id_schema("bt")}), &["tab_id"]),
        ),
        tool(
            "browser.tab.close",
            "Close one owned tab under the current control generation.",
            control_schema(json!({"tab_id": id_schema("bt")}), &["tab_id"]),
        ),
        tool(
            "browser.close",
            "Close an owned browser session, revoke control, and supervise cleanup under the current control generation.",
            control_schema(json!({}), &[]),
        ),
        tool(
            "browser.navigate",
            "Navigate the exact current document to an HTTP(S) URL within the authorized session boundary. Navigation may cause website effects.",
            document_schema(json!({"url": url_schema()}), &["url"]),
        ),
        tool(
            "browser.snapshot",
            "Read a bounded semantic snapshot of the current document. Returned element handles expire when the document or snapshot changes; sensitive fields are withheld.",
            document_schema(
                json!({"max_nodes": {"type": "integer", "minimum": 1, "maximum": 1024}}),
                &["max_nodes"],
            ),
        ),
        tool(
            "browser.screenshot",
            "Capture the current viewport into an owner-only PNG artifact after output policy permits release. Returns verified artifact metadata and supplies the image to an image-capable model automatically. Accepts no file path, raw protocol, or image bytes.",
            document_schema(json!({}), &[]),
        ),
        tool(
            "browser.click",
            "Click one fresh element from the current snapshot. A click is a website mutation and may submit data.",
            element_schema(json!({}), &[]),
        ),
        tool(
            "browser.upload",
            "Upload one existing owned RunInput artifact into a fresh ordinary file input. Policy inspects the actual bytes before website input. Accepts an opaque artifact ID, never a file path or encoded payload.",
            element_schema(
                json!({"artifact_id":{"type":"string","pattern":"^artifact-[0-9a-f]{64}$"}}),
                &["artifact_id"],
            ),
        ),
        tool(
            "browser.download",
            "Download one fresh link into an owner-only artifact after policy inspects the actual complete bytes. The browser selects private storage; unsolicited downloads and save dialogs remain blocked.",
            element_schema(json!({}), &[]),
        ),
        tool(
            "browser.fill",
            "Fill an ordinary permitted field using one fresh element handle. Passwords, credentials, certificate material, and other recognized secrets require protected native entry and are refused here.",
            element_schema(
                json!({"text": {"type": "string", "maxLength": 8192}}),
                &["text"],
            ),
        ),
        tool(
            "browser.select",
            "Choose bounded option values or unique visible option labels in a fresh referenced form control. Refresh the snapshot after each mutation. Option changes are website mutations.",
            element_schema(
                json!({"values": {
                    "type": "array", "minItems": 1, "maxItems": 32, "uniqueItems": true,
                    "items": {"type": "string", "maxLength": 1024}
                }}),
                &["values"],
            ),
        ),
        tool(
            "browser.press",
            "Send one permitted key to the current guest document. Keys may submit forms or trigger page handlers; app and browser accelerators are unavailable.",
            document_schema(
                json!({"key": {"type": "string", "enum": [
                    "enter", "tab", "escape", "backspace", "delete",
                    "arrow_up", "arrow_down", "arrow_left", "arrow_right",
                    "home", "end", "page_up", "page_down", "space"
                ]}}),
                &["key"],
            ),
        ),
        tool(
            "browser.scroll",
            "Scroll the current document by bounded horizontal and vertical distances. Scrolling may invoke page handlers and network effects.",
            document_schema(
                json!({
                    "x": {"type": "integer", "minimum": -10000, "maximum": 10000},
                    "y": {"type": "integer", "minimum": -10000, "maximum": 10000}
                }),
                &["x", "y"],
            ),
        ),
        tool(
            "browser.wait",
            "Wait for document load or visibility of one fresh referenced element under a bounded deadline; waiting never authorizes page input.",
            document_schema(
                json!({
                    "condition": {"oneOf": [
                        object_schema(json!({"kind": {"const": "load"}}), &["kind"]),
                        object_schema(json!({
                            "kind": {"const": "element_visible"},
                            "element": object_schema(json!({
                                "document_id": id_schema("bd"),
                                "snapshot_id": id_schema("bn"),
                                "element_id": id_schema("be")
                            }), &["document_id", "snapshot_id", "element_id"])
                        }), &["kind", "element"])
                    ]},
                    "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 30000}
                }),
                &["condition", "timeout_ms"],
            ),
        ),
    ];
    for (name, description) in [
        (
            "browser.back",
            "Navigate back in the exact current tab's history. History navigation may cause website effects.",
        ),
        (
            "browser.forward",
            "Navigate forward in the exact current tab's history. History navigation may cause website effects.",
        ),
        (
            "browser.reload",
            "Reload the exact current document. Reloading may repeat requests with website effects.",
        ),
        (
            "browser.stop",
            "Stop loading the exact current document under the current control generation. Stopping does not undo dispatched website effects.",
        ),
    ] {
        specs.push(tool(name, description, document_schema(json!({}), &[])));
    }
    specs.extend(interactions::specs());
    specs
}

fn tool(name: &str, description: &str, input_schema: Value) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema,
        effect_action: Some(name.into()),
        capability: Some(name.into()),
        max_output_bytes: if matches!(
            name,
            "browser.screenshot" | "browser.screenshot.annotated" | "browser.download"
        ) {
            4 * 1024 * 1024
        } else {
            64 * 1024
        },
    }
}

fn id_schema(prefix: &str) -> Value {
    json!({
        "type": "string", "minLength": 35, "maxLength": 35,
        "pattern": format!("^{prefix}_[0-9a-f]{{32}}$")
    })
}

fn url_schema() -> Value {
    json!({
        "type": "string", "minLength": 1, "maxLength": 4096,
        "pattern": "^https?://[^/@?#\\s]+(?:[/?#][^\\s]*)?$"
    })
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "properties": properties, "required": required
    })
}

fn control_schema(properties: Value, required: &[&str]) -> Value {
    let mut all = json!({
        "session_id": id_schema("bs"),
        "control_generation": {"type": "integer", "minimum": 1, "maximum": u64::MAX}
    });
    if let (Some(all), Some(properties)) = (all.as_object_mut(), properties.as_object()) {
        all.extend(properties.clone());
    }
    let mut fields = vec!["session_id", "control_generation"];
    fields.extend(required);
    object_schema(all, &fields)
}

fn document_schema(properties: Value, required: &[&str]) -> Value {
    let mut all = json!({"tab_id": id_schema("bt"), "document_id": id_schema("bd")});
    if let (Some(all), Some(properties)) = (all.as_object_mut(), properties.as_object()) {
        all.extend(properties.clone());
    }
    let mut fields = vec!["tab_id", "document_id"];
    fields.extend(required);
    control_schema(all, &fields)
}

fn element_schema(properties: Value, required: &[&str]) -> Value {
    let mut all = json!({"snapshot_id": id_schema("bn"), "element_id": id_schema("be")});
    if let (Some(all), Some(properties)) = (all.as_object_mut(), properties.as_object()) {
        all.extend(properties.clone());
    }
    let mut fields = vec!["snapshot_id", "element_id"];
    fields.extend(required);
    document_schema(all, &fields)
}
