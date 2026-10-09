use super::*;
use colossus_contracts::ToolSpec;

const BROWSER_ACTIONS: &[(&str, ActionClass)] = &[
    ("browser.open", ActionClass::ExternalNetwork),
    ("browser.status", ActionClass::Read),
    ("browser.tabs", ActionClass::Read),
    ("browser.tab.open", ActionClass::ExternalNetwork),
    ("browser.tab.select", ActionClass::LocalState),
    ("browser.tab.close", ActionClass::LocalState),
    ("browser.navigate", ActionClass::ExternalNetwork),
    ("browser.back", ActionClass::ExternalNetwork),
    ("browser.forward", ActionClass::ExternalNetwork),
    ("browser.reload", ActionClass::ExternalNetwork),
    ("browser.stop", ActionClass::LocalState),
    ("browser.snapshot", ActionClass::Read),
    ("browser.click", ActionClass::ExternalNetwork),
    ("browser.fill", ActionClass::ExternalNetwork),
    ("browser.select", ActionClass::ExternalNetwork),
    ("browser.press", ActionClass::ExternalNetwork),
    ("browser.scroll", ActionClass::ExternalNetwork),
    ("browser.wait", ActionClass::Read),
    ("browser.close", ActionClass::LocalState),
];

fn browser_specs() -> Vec<ToolSpec> {
    BROWSER_ACTIONS
        .iter()
        .map(|(name, _)| ToolSpec {
            name: (*name).into(),
            description: "test browser".into(),
            input_schema: serde_json::json!({"type": "object"}),
            effect_action: Some((*name).into()),
            capability: Some((*name).into()),
            max_output_bytes: 1,
        })
        .collect()
}

fn resolve(config: &AccessConfig, available: bool) -> AccessResolution {
    resolve_access(
        config,
        &browser_specs(),
        builtin_action_descriptors(),
        BROWSER_ACTIONS
            .iter()
            .map(|(name, _)| builtin_tool_descriptor(name).unwrap()),
        &AccessContext {
            browser_available: available,
            ..AccessContext::default()
        },
        false,
    )
    .expect("optional browser resolution")
}

#[test]
fn browser_requires_trusted_backend_even_when_explicitly_selected() {
    assert!(!AccessContext::default().browser_available);
    for config in [
        AccessConfig::default(),
        AccessConfig {
            profile: AccessProfile::Pinned,
            tools: ToolAccessConfig {
                include: vec!["browser.*".into()],
                ..ToolAccessConfig::default()
            },
            ..AccessConfig::default()
        },
        AccessConfig {
            profile: AccessProfile::Pinned,
            tools: ToolAccessConfig {
                include: BROWSER_ACTIONS
                    .iter()
                    .map(|(name, _)| (*name).into())
                    .collect(),
                ..ToolAccessConfig::default()
            },
            ..AccessConfig::default()
        },
    ] {
        let unavailable = resolve(&config, false);
        assert!(unavailable.active_tool_names().is_empty());
        for entry in unavailable.tools {
            assert_eq!(
                entry.unmet_prerequisite.as_deref(),
                Some("verified browser backend")
            );
            assert_eq!(entry.family, "browser");
            assert_eq!(entry.source, CapabilitySource::Core);
        }
        assert_eq!(
            resolve(&config, true).active_tool_names().len(),
            BROWSER_ACTIONS.len()
        );
    }
}

#[test]
fn browser_availability_never_grants_page_mutation_authority() {
    let descriptors = builtin_action_descriptors();
    let config = AccessConfig {
        profile: AccessProfile::Development,
        ..AccessConfig::default()
    };
    let resolution = resolve(&config, true);
    for (name, class) in BROWSER_ACTIONS {
        let descriptor = descriptors
            .iter()
            .find(|entry| entry.name == *name)
            .expect("explicit browser classification");
        assert_eq!(descriptor.class, *class, "classification {name}");
        assert_eq!(descriptor.source, CapabilitySource::Core);
        let expected = if *class == ActionClass::ExternalNetwork {
            AccessDecision::RequireApproval
        } else {
            AccessDecision::Allow
        };
        assert_eq!(
            resolution.action_decision(name),
            Some(expected),
            "authority {name}"
        );
    }
    let pinned = resolve(
        &AccessConfig {
            profile: AccessProfile::Pinned,
            tools: ToolAccessConfig {
                include: vec!["browser.*".into()],
                ..ToolAccessConfig::default()
            },
            ..AccessConfig::default()
        },
        true,
    );
    assert_eq!(
        pinned.action_decision("browser.press"),
        Some(AccessDecision::Deny)
    );
    assert_eq!(
        pinned.action_decision("browser.scroll"),
        Some(AccessDecision::Deny)
    );
}
