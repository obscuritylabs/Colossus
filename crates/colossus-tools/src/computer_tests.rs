use super::*;

fn validate(name: &str, arguments: Value) -> Result<ToolSpec, ToolError> {
    StaticToolRegistry::builtins(&[name.into()])
        .unwrap()
        .validate(&ToolCall {
            call_id: "computer-schema".into(),
            name: name.into(),
            arguments,
        })
}

#[test]
fn computer_schemas_have_exact_effects_and_require_observation_for_input() {
    for spec in builtin_specs()
        .into_iter()
        .filter(|spec| spec.name.starts_with("computer."))
    {
        assert_eq!(spec.effect_action.as_deref(), Some(spec.name.as_str()));
        assert_eq!(spec.capability.as_deref(), Some(spec.name.as_str()));
        assert_eq!(spec.input_schema["additionalProperties"], false);
    }
    let envelope = json!({"session_id":format!("cu_s_{}","a".repeat(32)),"control_generation":1,
        "observation_id":format!("cu_o_{}","a".repeat(32)),"keys":["Control","A"]});
    assert!(validate("computer.press", envelope.clone()).is_ok());
    for key in ["observation_id", "session_id", "control_generation"] {
        let mut missing = envelope.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(validate("computer.press", missing).is_err());
    }
    let mut forged = envelope;
    forged["hwnd"] = json!(123);
    assert!(validate("computer.press", forged).is_err());
    assert!(validate("computer.status", json!({})).is_ok());
    assert!(
        validate(
            "computer.open",
            json!({"window_id":format!("cu_w_{}","a".repeat(32)),"enabled":true})
        )
        .is_err()
    );
}

#[test]
fn computer_schemas_reject_native_ids_cross_kind_ids_and_ambient_code() {
    for arguments in [
        json!({"window_id":123}),
        json!({"window_id":format!("cu_s_{}","a".repeat(32))}),
        json!({"window_id":format!("cu_w_{}","a".repeat(33))}),
        json!({"executable":"powershell.exe"}),
    ] {
        assert!(validate("computer.open", arguments).is_err());
    }
    assert!(validate("computer.windows", json!({"max_windows":129})).is_err());
    assert!(validate("computer.windows", json!({"max_windows":0})).is_err());
}
