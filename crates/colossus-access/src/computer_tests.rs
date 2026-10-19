use super::*;
use colossus_contracts::ToolSpec;

#[test]
fn computer_host_prerequisite_cannot_be_manufactured_by_allow_all_or_explicit_selection() {
    let names = [
        "computer.windows",
        "computer.open",
        "computer.snapshot",
        "computer.screenshot",
        "computer.status",
        "computer.focus",
        "computer.move",
        "computer.click",
        "computer.drag",
        "computer.scroll",
        "computer.type",
        "computer.press",
        "computer.close",
    ];
    let specs = names
        .iter()
        .map(|name| ToolSpec {
            name: (*name).into(),
            description: "native".into(),
            input_schema: serde_json::json!({"type":"object"}),
            effect_action: Some((*name).into()),
            capability: Some((*name).into()),
            max_output_bytes: 1,
        })
        .collect::<Vec<_>>();
    let config = AccessConfig {
        profile: AccessProfile::AllowAll,
        tools: ToolAccessConfig {
            include: names.iter().map(|name| (*name).into()).collect(),
            ..Default::default()
        },
        ..Default::default()
    };
    for available in [false, true] {
        let result = resolve_access(
            &config,
            &specs,
            builtin_action_descriptors(),
            names
                .iter()
                .map(|name| builtin_tool_descriptor(name).unwrap()),
            &AccessContext {
                computer_available: available,
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(
            result.active_tool_names().len(),
            if available { names.len() } else { 0 }
        );
        assert_eq!(
            result.action_decision("computer.click"),
            Some(AccessDecision::Allow)
        );
    }
    let descriptors = builtin_action_descriptors();
    for (name, class) in [
        ("computer.screenshot", ActionClass::Read),
        ("computer.snapshot", ActionClass::Read),
        ("computer.open", ActionClass::LocalState),
        ("computer.close", ActionClass::LocalState),
        ("computer.click", ActionClass::Execution),
        ("computer.type", ActionClass::Execution),
    ] {
        assert_eq!(
            descriptors
                .iter()
                .find(|item| item.name == name)
                .unwrap()
                .class,
            class
        );
    }
}
