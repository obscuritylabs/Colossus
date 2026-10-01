//! Portable credential labels and kinds; no secret values or host identities.
use super::{
    globals::{self, SetupGlobals},
    types::invalid,
};
use crate::{
    dto::CommandErrorDto,
    managed_configuration::{CredentialKindSetting, CredentialMetadataSetting},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PortableCredential {
    pub label: String,
    pub kind: CredentialKindSetting,
}

pub(super) fn validate(
    metadata: &BTreeMap<String, PortableCredential>,
    slots: &BTreeSet<String>,
) -> Result<(), CommandErrorDto> {
    if metadata.len() > 128
        || metadata.iter().any(|(slot, value)| {
            !slots.contains(slot)
                || value.label.trim().is_empty()
                || value.label.len() > 96
                || value.label.chars().any(char::is_control)
        })
    {
        return Err(invalid(
            "Credential descriptions need a referenced env:NAME slot, a name of at most 96 bytes, and a supported kind.",
        ));
    }
    Ok(())
}

pub(super) fn for_export(
    source: Option<&CredentialMetadataSetting>,
    fallback: &str,
) -> PortableCredential {
    PortableCredential {
        label: source.map_or_else(|| bounded_label(fallback), |source| source.label.clone()),
        kind: source.map_or(CredentialKindSetting::GenericSecret, |source| source.kind),
    }
}

fn bounded_label(value: &str) -> String {
    let mut end = value.len().min(96);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

pub(super) fn unique_slot(label: &str, used: &BTreeMap<String, PortableCredential>) -> String {
    let name = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let name = name.trim_matches('_');
    let base = format!(
        "env:COLOSSUS_{}",
        if name.is_empty() { "CREDENTIAL" } else { name }
    );
    let mut slot = base.clone();
    let mut suffix = 2;
    while used.contains_key(&slot) {
        slot = format!("{base}_{suffix}");
        suffix += 1;
    }
    slot
}

pub(super) fn missing(globals: &SetupGlobals, slot: &str, id: &str) -> CredentialMetadataSetting {
    let mut credential = globals::missing_credential(slot, id);
    if let Some(metadata) = globals.credentials.get(slot) {
        credential.label.clone_from(&metadata.label);
        credential.kind = metadata.kind;
    }
    credential
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_slots_distinguish_colliding_names_and_bound_unicode() {
        let mut used = BTreeMap::new();
        for name in ["Docs token", "Docs-token", "123 key", "🔑", &"é".repeat(48)] {
            let value = for_export(None, name);
            let slot = unique_slot(&value.label, &used);
            assert!(super::super::configuration::valid_slot(&slot));
            assert!(used.insert(slot, value).is_none());
        }
        assert!(used.contains_key("env:COLOSSUS_DOCS_TOKEN"));
        assert!(used.contains_key("env:COLOSSUS_DOCS_TOKEN_2"));
    }

    #[test]
    fn metadata_cannot_include_tokens_or_unreferenced_slots() {
        let mut globals = SetupGlobals::default();
        globals
            .credentials
            .insert("env:UNUSED".into(), for_export(None, "Unused"));
        assert!(globals.validate().is_err());
        assert!(
            serde_json::from_value::<PortableCredential>(serde_json::json!({
                "label":"Docs", "kind":"bearer_token", "value":"must-not-import"
            }))
            .is_err()
        );
        assert!(
            validate(
                &BTreeMap::from([("env:X".into(), for_export(None, "bad\nlabel"))]),
                &BTreeSet::from(["env:X".into()])
            )
            .is_err()
        );
    }
}
