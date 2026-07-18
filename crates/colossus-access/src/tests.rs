use super::*;
use colossus_contracts::ToolSpec;

fn tool(name: &str, action: Option<&str>) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: "test".into(),
        input_schema: serde_json::json!({"type": "object"}),
        effect_action: action.map(str::to_owned),
        capability: action.map(str::to_owned),
        max_output_bytes: 1,
    }
}

fn core_tool(name: &str) -> ToolDescriptor {
    builtin_tool_descriptor(name).expect("built-in tool descriptor")
}

#[test]
fn profiles_apply_to_synthetic_future_capabilities_by_metadata() {
    let tools = vec![
        tool("echo", None),
        tool("future.read", Some("future.read")),
        tool("future.write", Some("future.write")),
    ];
    let tool_descriptors = vec![
        core_tool("echo"),
        ToolDescriptor::new("future.read", "future", CapabilitySource::Core, Vec::new()),
        ToolDescriptor::new("future.write", "future", CapabilitySource::Core, Vec::new()),
    ];
    let descriptors = [
        ActionDescriptor::new(
            "provider.echo",
            ActionClass::Provider,
            CapabilitySource::Core,
        ),
        ActionDescriptor::new("future.read", ActionClass::Read, CapabilitySource::Core),
        ActionDescriptor::new(
            "future.write",
            ActionClass::WorkspaceMutation,
            CapabilitySource::Core,
        ),
    ];
    let resolution = resolve_access(
        &AccessConfig::default(),
        &tools,
        descriptors,
        tool_descriptors.clone(),
        &AccessContext::default(),
        false,
    )
    .expect("development resolution");
    assert_eq!(
        resolution.action_decision("future.read"),
        Some(AccessDecision::Allow)
    );
    assert_eq!(
        resolution.action_decision("future.write"),
        Some(AccessDecision::RequireApproval)
    );
    assert_eq!(resolution.active_tool_names().len(), 3);

    let minimal = resolve_access(
        &AccessConfig {
            profile: AccessProfile::Minimal,
            ..AccessConfig::default()
        },
        &tools,
        [
            ActionDescriptor::new("future.read", ActionClass::Read, CapabilitySource::Core),
            ActionDescriptor::new(
                "future.write",
                ActionClass::WorkspaceMutation,
                CapabilitySource::Core,
            ),
        ],
        tool_descriptors,
        &AccessContext::default(),
        false,
    )
    .expect("minimal resolution");
    assert_eq!(minimal.active_tool_names(), ["echo"]);
}

#[test]
fn every_profile_has_the_documented_action_matrix() {
    let descriptors = [
        ActionDescriptor::new(
            "provider.echo",
            ActionClass::Provider,
            CapabilitySource::Core,
        ),
        ActionDescriptor::new(
            "provider.test",
            ActionClass::Provider,
            CapabilitySource::Integration,
        ),
        ActionDescriptor::new(
            "read.test",
            ActionClass::Read,
            CapabilitySource::Integration,
        ),
        ActionDescriptor::new(
            "state.test",
            ActionClass::LocalState,
            CapabilitySource::Integration,
        ),
        ActionDescriptor::new(
            "write.test",
            ActionClass::WorkspaceMutation,
            CapabilitySource::Integration,
        ),
        ActionDescriptor::new(
            "execute.test",
            ActionClass::Execution,
            CapabilitySource::Integration,
        ),
        ActionDescriptor::new(
            "network.test",
            ActionClass::ExternalNetwork,
            CapabilitySource::Integration,
        ),
        ActionDescriptor::new(
            "admin.test",
            ActionClass::Administration,
            CapabilitySource::Integration,
        ),
    ];
    let resolve = |profile| {
        resolve_access(
            &AccessConfig {
                profile,
                ..AccessConfig::default()
            },
            &[tool("echo", None)],
            descriptors.clone(),
            [core_tool("echo")],
            &AccessContext::default(),
            false,
        )
        .expect("profile resolution")
    };
    let minimal = resolve(AccessProfile::Minimal);
    assert_eq!(
        minimal.action_decision("provider.test"),
        Some(AccessDecision::Allow)
    );
    assert!(
        descriptors
            .iter()
            .filter(|descriptor| descriptor.class != ActionClass::Provider)
            .all(|descriptor| {
                minimal.action_decision(&descriptor.name) == Some(AccessDecision::Deny)
            })
    );

    let development = resolve(AccessProfile::Development);
    for action in ["provider.echo", "provider.test", "read.test", "state.test"] {
        assert_eq!(
            development.action_decision(action),
            Some(AccessDecision::Allow)
        );
    }
    for action in ["write.test", "execute.test", "network.test", "admin.test"] {
        assert_eq!(
            development.action_decision(action),
            Some(AccessDecision::RequireApproval)
        );
    }

    let allow_all = resolve(AccessProfile::AllowAll);
    assert!(descriptors.iter().all(|descriptor| {
        allow_all.action_decision(&descriptor.name) == Some(AccessDecision::Allow)
    }));

    let pinned = resolve(AccessProfile::Pinned);
    assert_eq!(
        pinned.action_decision("provider.echo"),
        Some(AccessDecision::Allow)
    );
    assert!(
        descriptors
            .iter()
            .filter(|descriptor| descriptor.name != "provider.echo")
            .all(|descriptor| {
                pinned.action_decision(&descriptor.name) == Some(AccessDecision::Deny)
            })
    );
}

#[test]
fn action_overrides_are_exclusive_and_opa_owns_outcomes() {
    let mut config = AccessConfig::default();
    config.actions.allow.push("web.search".into());
    config.actions.deny.push("web.search".into());
    assert!(validate_config(&config, false).is_err());
    config.actions.deny.clear();
    assert!(validate_config(&config, true).is_err());
}

#[test]
fn wildcard_selects_tools_but_action_wildcards_fail() {
    let mut config = AccessConfig {
        profile: AccessProfile::Pinned,
        ..AccessConfig::default()
    };
    config.tools.include.push("*".into());
    let resolution = resolve_access(
        &config,
        &[
            tool("echo", None),
            tool("filesystem.read", Some("filesystem.read")),
        ],
        builtin_action_descriptors(),
        [core_tool("echo"), core_tool("filesystem.read")],
        &AccessContext::default(),
        false,
    )
    .expect("wildcard");
    assert_eq!(resolution.active_tool_names(), ["echo"]);
    assert_eq!(
        resolution
            .tools
            .iter()
            .find(|tool| tool.name == "filesystem.read")
            .expect("filesystem read")
            .unmet_prerequisite
            .as_deref(),
        Some("filesystem read grant")
    );
    config.actions.allow.push("*".into());
    assert!(validate_config(&config, false).is_err());
}

#[test]
fn explicit_tool_inclusion_does_not_grant_its_action() {
    let config = AccessConfig {
        profile: AccessProfile::Pinned,
        tools: ToolAccessConfig {
            include: vec!["future.write".into()],
            exclude: Vec::new(),
        },
        actions: ActionAccessConfig::default(),
    };
    let resolution = resolve_access(
        &config,
        &[tool("future.write", Some("future.write"))],
        [ActionDescriptor::new(
            "future.write",
            ActionClass::WorkspaceMutation,
            CapabilitySource::SignedPack,
        )],
        [ToolDescriptor::new(
            "future.write",
            "future",
            CapabilitySource::SignedPack,
            Vec::new(),
        )],
        &AccessContext::default(),
        false,
    )
    .expect("pinned extension");
    assert_eq!(resolution.active_tool_names(), ["future.write"]);
    assert_eq!(
        resolution.action_decision("future.write"),
        Some(AccessDecision::Deny)
    );
}

#[test]
fn inherited_prerequisites_hide_and_explicit_prerequisites_fail() {
    let specs = [tool("filesystem.read", Some("filesystem.read"))];
    let inherited = resolve_access(
        &AccessConfig::default(),
        &specs,
        builtin_action_descriptors(),
        [core_tool("filesystem.read")],
        &AccessContext::default(),
        false,
    )
    .expect("inherited tool is diagnosable");
    assert_eq!(inherited.tools[0].availability, ToolAvailability::Hidden);
    assert_eq!(
        inherited.tools[0].unmet_prerequisite.as_deref(),
        Some("filesystem read grant")
    );

    let explicitly_included = AccessConfig {
        tools: ToolAccessConfig {
            include: vec!["filesystem.read".into()],
            exclude: Vec::new(),
        },
        ..AccessConfig::default()
    };
    assert!(matches!(
        resolve_access(
            &explicitly_included,
            &specs,
            builtin_action_descriptors(),
            [core_tool("filesystem.read")],
            &AccessContext::default(),
            false,
        ),
        Err(AccessError::Invalid(_))
    ));
}

#[test]
fn built_in_action_descriptors_are_unique_and_deterministic() {
    let descriptors = builtin_action_descriptors();
    assert!(
        descriptors
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
    );
}

#[test]
fn unknown_core_tool_fails_closed() {
    let error = resolve_access(
        &AccessConfig::default(),
        &[tool("future.core", None)],
        builtin_action_descriptors(),
        Vec::<ToolDescriptor>::new(),
        &AccessContext::default(),
        false,
    )
    .expect_err("unclassified");
    assert!(matches!(error, AccessError::Unclassified(_)));
}
