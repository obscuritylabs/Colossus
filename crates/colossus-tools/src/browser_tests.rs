use super::*;

fn opaque(prefix: &str) -> String {
    format!("{prefix}_{}", "a".repeat(32))
}

fn control() -> Value {
    json!({"session_id": opaque("bs"), "control_generation": 1})
}

fn document() -> Value {
    extend(
        control(),
        json!({"tab_id": opaque("bt"), "document_id": opaque("bd")}),
    )
}

fn element() -> Value {
    extend(
        document(),
        json!({"snapshot_id": opaque("bn"), "element_id": opaque("be")}),
    )
}

fn extend(mut base: Value, fields: Value) -> Value {
    base.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    base
}

fn validate(name: &str, arguments: Value) -> Result<ToolSpec, ToolError> {
    StaticToolRegistry::builtins(&[name.into()])
        .expect("browser schema")
        .validate(&ToolCall {
            call_id: "browser-test".into(),
            name: name.into(),
            arguments,
        })
}

#[test]
fn browser_tools_accept_typed_operations_and_have_exact_effect_identity() {
    let cases = [
        (
            "browser.open",
            json!({"mode": "embedded", "allowed_origins": ["https://example.test"], "initial_url": "https://example.test/start"}),
        ),
        ("browser.status", json!({"session_id": opaque("bs")})),
        ("browser.tabs", json!({"session_id": opaque("bs")})),
        (
            "browser.tab.open",
            extend(control(), json!({"url": "https://example.test"})),
        ),
        (
            "browser.tab.select",
            extend(control(), json!({"tab_id": opaque("bt")})),
        ),
        (
            "browser.tab.close",
            extend(control(), json!({"tab_id": opaque("bt")})),
        ),
        ("browser.close", control()),
        (
            "browser.navigate",
            extend(document(), json!({"url": "https://example.test/next"})),
        ),
        ("browser.back", document()),
        ("browser.forward", document()),
        ("browser.reload", document()),
        ("browser.stop", document()),
        (
            "browser.snapshot",
            extend(document(), json!({"max_nodes": 1024})),
        ),
        ("browser.click", element()),
        (
            "browser.fill",
            extend(element(), json!({"text": "ordinary text"})),
        ),
        (
            "browser.select",
            extend(element(), json!({"values": ["first"]})),
        ),
        ("browser.press", extend(document(), json!({"key": "enter"}))),
        (
            "browser.scroll",
            extend(document(), json!({"x": -10000, "y": 10000})),
        ),
        (
            "browser.wait",
            extend(
                document(),
                json!({"condition": {"kind": "load"}, "timeout_ms": 30000}),
            ),
        ),
    ];
    for (name, arguments) in cases {
        let spec = validate(name, arguments).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(spec.effect_action.as_deref(), Some(name));
        assert_eq!(spec.capability.as_deref(), Some(name));
        assert_eq!(spec.max_output_bytes, 64 * 1024);
    }
    assert!(
        validate(
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://example.test"]})
        )
        .is_ok()
    );
    assert!(validate("browser.tab.open", control()).is_ok());
    assert!(validate("browser.wait", extend(document(), json!({
        "condition": {"kind": "element_visible", "element": {
            "document_id": opaque("bd"), "snapshot_id": opaque("bn"), "element_id": opaque("be")
        }}, "timeout_ms": 1
    }))).is_ok());
}

#[test]
fn browser_schemas_reject_protocol_escape_credentials_and_unowned_handle_shapes() {
    for (name, arguments) in [
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://example.test"], "executable": "/tmp/chrome"}),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://example.test"], "profile_path": "/tmp/profile"}),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://example.test"], "certificate_password": "value"}),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["*"], "initial_url": "https://example.test"}),
        ),
        (
            "browser.navigate",
            extend(document(), json!({"url": "javascript:alert(1)"})),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://example.test/path"]}),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://example.test?token=secret"]}),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://user:password@example.test"]}),
        ),
        (
            "browser.navigate",
            extend(
                document(),
                json!({"url": "https://user:password@example.test"}),
            ),
        ),
        (
            "browser.navigate",
            extend(document(), json!({"url": "file:///tmp/secret"})),
        ),
        (
            "browser.click",
            extend(element(), json!({"cdp_target_id": "target-1"})),
        ),
        (
            "browser.click",
            extend(element(), json!({"element_id": "#submit"})),
        ),
        (
            "browser.click",
            extend(element(), json!({"snapshot_id": opaque("bs")})),
        ),
        (
            "browser.fill",
            extend(element(), json!({"text": "input", "password": "secret"})),
        ),
        (
            "browser.press",
            extend(document(), json!({"key": "Ctrl+L"})),
        ),
        (
            "browser.wait",
            extend(
                document(),
                json!({"condition": {"kind": "javascript", "expression": "true"}, "timeout_ms": 100}),
            ),
        ),
        (
            "browser.wait",
            extend(
                document(),
                json!({"condition": {"kind": "load", "expression": "true"}, "timeout_ms": 100}),
            ),
        ),
        (
            "browser.wait",
            extend(
                document(),
                json!({"condition": {"kind": "element_visible", "element": {
            "document_id": opaque("bd"), "snapshot_id": opaque("bn"), "element_id": opaque("be"), "selector": "#submit"
        }}, "timeout_ms": 100}),
            ),
        ),
    ] {
        assert!(
            matches!(
                validate(name, arguments),
                Err(ToolError::InvalidArguments { .. })
            ),
            "reject {name}"
        );
    }
    for name in [
        "browser.evaluate",
        "browser.cdp",
        "browser.screenshot",
        "browser.upload",
    ] {
        assert!(
            StaticToolRegistry::builtins(&[name.into()]).is_err(),
            "unsupported {name}"
        );
    }
}

#[test]
fn browser_schemas_require_fresh_handle_fields_and_enforce_resource_bounds() {
    for key in [
        "session_id",
        "control_generation",
        "tab_id",
        "document_id",
        "snapshot_id",
        "element_id",
    ] {
        let mut arguments = element();
        arguments.as_object_mut().unwrap().remove(key);
        assert!(
            matches!(
                validate("browser.click", arguments),
                Err(ToolError::InvalidArguments { .. })
            ),
            "requires {key}"
        );
    }
    for (name, arguments) in [
        (
            "browser.close",
            extend(control(), json!({"control_generation": 0})),
        ),
        (
            "browser.close",
            extend(control(), json!({"control_generation": 1.5})),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": []}),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://example.test", "https://example.test"]}),
        ),
        (
            "browser.snapshot",
            extend(document(), json!({"max_nodes": 1025})),
        ),
        (
            "browser.fill",
            extend(element(), json!({"text": "x".repeat(8193)})),
        ),
        ("browser.select", extend(element(), json!({"values": []}))),
        (
            "browser.select",
            extend(element(), json!({"values": ["x".repeat(1025)]})),
        ),
        (
            "browser.scroll",
            extend(document(), json!({"x": 0, "y": 10001})),
        ),
        (
            "browser.wait",
            extend(
                document(),
                json!({"condition": {"kind": "load"}, "timeout_ms": 30001}),
            ),
        ),
        (
            "browser.wait",
            extend(
                document(),
                json!({"condition": {"kind": "load"}, "timeout_ms": 0}),
            ),
        ),
    ] {
        assert!(
            matches!(
                validate(name, arguments),
                Err(ToolError::InvalidArguments { .. })
            ),
            "bound {name}"
        );
    }
}

#[test]
fn browser_validation_errors_never_echo_rejected_credentials_or_page_text() {
    let private = "browser-private-marker";
    for (name, arguments) in [
        (
            "browser.navigate",
            extend(
                document(),
                json!({"url": format!("https://user:{private}@example.test")}),
            ),
        ),
        (
            "browser.fill",
            extend(element(), json!({"text": private.repeat(8192)})),
        ),
        (
            "browser.open",
            json!({"mode": "headless", "allowed_origins": ["https://example.test"], "password": private}),
        ),
    ] {
        let error = validate(name, arguments).expect_err("invalid browser input");
        assert!(
            !error.to_string().contains(private),
            "rejected input remains private"
        );
        assert!(matches!(error, ToolError::InvalidArguments { .. }));
    }
}
