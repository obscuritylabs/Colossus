//! Bounded semantic translation. Raw protocol data never leaves the native host.
use std::collections::HashMap;

use colossus_contracts::{
    BrowserDocumentId, BrowserElementId, BrowserElementRef, BrowserSnapshot, BrowserSnapshotId,
    BrowserSnapshotNode,
};
use colossus_ports::BrowserDriverError;
use serde_json::Value;

pub struct Candidate {
    pub backend: i32,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub protected: bool,
}

fn text(value: &Value, key: &str, limit: usize) -> String {
    bounded(
        value
            .get(key)
            .and_then(|value| value.get("value"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        limit,
    )
}

pub fn bounded(value: &str, limit: usize) -> String {
    let mut result = String::new();
    for character in value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
    {
        if result.len() + character.len_utf8() > limit {
            break;
        }
        result.push(character);
    }
    result
}

pub fn candidates(
    value: &Value,
    max_nodes: u16,
) -> Result<(Vec<Candidate>, bool), BrowserDriverError> {
    let nodes = value
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or(BrowserDriverError::Failed)?;
    if !(1..=1024).contains(&max_nodes) {
        return Err(BrowserDriverError::LimitExceeded);
    }
    let mut result = Vec::new();
    let mut truncated = false;
    for node in nodes {
        if node.get("ignored").and_then(Value::as_bool).unwrap_or(true) {
            continue;
        }
        let Some(backend) = node
            .get("backendDOMNodeId")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .filter(|value| *value > 0)
        else {
            continue;
        };
        if result.len() == usize::from(max_nodes) {
            truncated = true;
            break;
        }
        let protected = node
            .get("properties")
            .and_then(Value::as_array)
            .is_some_and(|properties| {
                properties.iter().any(|property| {
                    property.get("name").and_then(Value::as_str) == Some("protected")
                        && property
                            .get("value")
                            .and_then(|value| value.get("value"))
                            .and_then(Value::as_bool)
                            == Some(true)
                })
            });
        result.push(Candidate {
            backend,
            role: text(node, "role", 128),
            name: text(node, "name", 1024),
            value: (!protected)
                .then(|| text(node, "value", 2048))
                .filter(|value| !value.is_empty()),
            protected,
        });
    }
    Ok((result, truncated))
}

pub fn protected(attributes: &[String]) -> bool {
    attributes.chunks_exact(2).any(|pair| {
        let key = pair[0].to_ascii_lowercase();
        let value = pair[1].to_ascii_lowercase();
        (key == "type" && pair[1].eq_ignore_ascii_case("password"))
            || (key == "autocomplete"
                && value.split_ascii_whitespace().any(|part| {
                    matches!(
                        part,
                        "current-password"
                            | "new-password"
                            | "one-time-code"
                            | "cc-number"
                            | "cc-csc"
                    )
                }))
            || (matches!(key.as_str(), "name" | "id")
                && ["password", "secret", "token", "credential"]
                    .iter()
                    .any(|part| value.contains(part)))
    })
}

fn element_id() -> Result<BrowserElementId, BrowserDriverError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| BrowserDriverError::Unavailable)?;
    let mut value = String::from("be_");
    use std::fmt::Write as _;
    for byte in bytes {
        write!(value, "{byte:02x}").map_err(|_| BrowserDriverError::Failed)?;
    }
    BrowserElementId::parse(value).map_err(|_| BrowserDriverError::Failed)
}

pub fn snapshot(
    candidates: Vec<Candidate>,
    attributes: &HashMap<i32, Vec<String>>,
    document: BrowserDocumentId,
    identity: BrowserSnapshotId,
    truncated: bool,
) -> Result<(BrowserSnapshot, HashMap<BrowserElementId, i32>), BrowserDriverError> {
    let mut result = BrowserSnapshot {
        snapshot_id: identity,
        document_id: document,
        nodes: Vec::new(),
        truncated,
    };
    let mut mapping = HashMap::new();
    let mut bytes = 256;
    for candidate in candidates {
        let element_id = element_id()?;
        let guarded = candidate.protected
            || attributes
                .get(&candidate.backend)
                .is_none_or(|attributes| protected(attributes));
        let node = BrowserSnapshotNode {
            element: BrowserElementRef {
                document_id: result.document_id.clone(),
                snapshot_id: result.snapshot_id.clone(),
                element_id: element_id.clone(),
            },
            role: candidate.role,
            name: candidate.name,
            value: (!guarded).then_some(candidate.value).flatten(),
        };
        let encoded = serde_json::to_vec(&node).map_err(|_| BrowserDriverError::Failed)?;
        if bytes + encoded.len() > 60 * 1024 {
            result.truncated = true;
            break;
        }
        bytes += encoded.len() + 1;
        mapping.insert(element_id, candidate.backend);
        result.nodes.push(node);
    }
    Ok((result, mapping))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn protected_fields_never_release_values_even_if_ax_omits_protection() {
        let candidates = candidates(&json!({"nodes":[{"ignored":false,"backendDOMNodeId":9,"role":{"value":"textbox"},"name":{"value":"Sign in"},"value":{"value":"sensitive-fixture"}}]}), 1).unwrap().0;
        let attributes = HashMap::from([(9, vec!["type".into(), "PASSWORD".into()])]);
        let (snapshot, mapping) = snapshot(
            candidates,
            &attributes,
            BrowserDocumentId::parse(format!("bd_{}", "1".repeat(32))).unwrap(),
            BrowserSnapshotId::parse(format!("bn_{}", "2".repeat(32))).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(snapshot.nodes[0].value, None);
        assert_eq!(mapping.len(), 1);
        assert!(
            !serde_json::to_string(&snapshot)
                .unwrap()
                .contains("sensitive-fixture")
        );
    }

    #[test]
    fn missing_attribute_evidence_withholds_value_and_bounds_nodes() {
        let tree = json!({"nodes":[{"ignored":false,"backendDOMNodeId":1,"role":{"value":"textbox"},"value":{"value":"private"}}, {"ignored":false,"backendDOMNodeId":2}]});
        let (nodes, truncated) = candidates(&tree, 1).unwrap();
        let (value, _) = snapshot(
            nodes,
            &HashMap::new(),
            BrowserDocumentId::parse(format!("bd_{}", "1".repeat(32))).unwrap(),
            BrowserSnapshotId::parse(format!("bn_{}", "2".repeat(32))).unwrap(),
            truncated,
        )
        .unwrap();
        assert!(value.truncated);
        assert!(value.nodes[0].value.is_none());
    }

    #[test]
    fn credential_attributes_and_utf8_bounds_are_conservative() {
        for attributes in [
            vec![
                "autocomplete".into(),
                "section-user current-password".into(),
            ],
            vec!["name".into(), "apiToken".into()],
        ] {
            assert!(protected(&attributes));
        }
        assert!(!protected(&["type".into(), "search".into()]));
        assert_eq!(bounded("🦀🦀x\0", 7), "🦀");
    }
}
