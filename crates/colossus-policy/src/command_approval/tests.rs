use super::*;
use crate::{effect_request, system_actor};
use colossus_contracts::CommandIntent;
use serde_json::json;

fn request() -> EffectRequest {
    let mut request = effect_request(
        system_actor("test"),
        "shell.run",
        "/bin/sh",
        json!({
            "cwd": "/work/project", "args": ["-c", "cargo test"],
            "environment": {"API_TOKEN": "unique-secret-value"}, "stdin_base64": "private-input"
        }),
    );
    request.command_intent = Some(CommandIntent {
        justification: "Check the build failure.".into(),
    });
    request
}

#[test]
fn projects_prepared_invocation_without_changing_execution() {
    let request = request();
    let original = request.clone();
    let context = command_approval_context(&request).unwrap().unwrap();
    assert_eq!(context.arguments, ["-c", "cargo test"]);
    assert_eq!(context.working_directory, "/work/project");
    assert_eq!(request, original);
    let json = serde_json::to_string(&context).unwrap();
    assert!(!json.contains("unique-secret-value"));
    assert!(!json.contains("private-input"));
    assert!(!context.redacted);
}

#[test]
fn redacts_credentials_in_shell_strings_urls_and_argument_pairs() {
    let mut request = request();
    request.content["args"] = json!([
        "-c",
        "TOKEN='secret value'; curl -H 'Authorization: Bearer abc123' 'https://user:pass@host/a?token=abc&ok=1'; echo unique-secret-value",
        "--password",
        "argument-secret"
    ]);
    let context = command_approval_context(&request).unwrap().unwrap();
    assert!(context.redacted);
    let text = serde_json::to_string(&context).unwrap();
    for secret in [
        "secret value",
        "abc123",
        "user:pass",
        "token=abc",
        "unique-secret-value",
        "argument-secret",
    ] {
        assert!(!text.contains(secret), "leaked {secret}");
    }
}

#[test]
fn url_credentials_are_masked_through_the_last_authority_delimiter() {
    let mut request = request();
    request.content["args"] = json!([
        "https://user:p@ss@host/path?ordinary=value#anchor",
        "curl 'https://user:p@ss@host/' https://example.test/path@name?q=x@y#z@w",
        "https://user:p@ss@[::1]:443/path",
        "https://example.test/?ordinary=user@host",
        "https://user:pa\"ss\"@host/path",
        r"https://user:p\@ss@host/path"
    ]);
    let original = request.clone();
    let context = command_approval_context(&request).unwrap().unwrap();
    assert_eq!(
        context.arguments,
        [
            "https://[REDACTED]@host/path?ordinary=value#anchor",
            "curl 'https://[REDACTED]@host/' https://example.test/path@name?q=x@y#z@w",
            "https://[REDACTED]@[::1]:443/path",
            "https://example.test/?ordinary=user@host",
            "https://[REDACTED]@host/path",
            "https://[REDACTED]@host/path"
        ]
    );
    assert!(context.redacted);
    assert_eq!(request, original);
}

#[test]
fn repeated_short_secrets_have_input_bounded_display_memory() {
    let mut request = request();
    request.content["environment"] = serde_json::Value::Object(
        (0..128)
            .map(|index| (format!("TOKEN_{index}"), json!("a")))
            .collect(),
    );
    request.content["args"] = json!(["a".repeat(65_536)]);
    let original = request.clone();
    let context = command_approval_context(&request).unwrap().unwrap();
    assert_eq!(context.arguments, ["[REDACTED]"]);
    assert!(context.redacted);
    assert_eq!(request, original);
    // Overlapping recognizers and multibyte values share the same byte mask.
    assert_eq!(
        sanitized("ééé END", &["é", "éé"]),
        ("[REDACTED] END".into(), true)
    );
}

#[test]
fn long_details_are_not_truncated_and_controls_are_visible() {
    let mut request = request();
    request.content["args"] = json!([format!("{}\n\u{1b}\u{202e}TAIL", "é".repeat(65500))]);
    let context = command_approval_context(&request).unwrap().unwrap();
    assert!(context.arguments[0].ends_with("\\n\\u{1b}\\u{202e}TAIL"));
    assert!(context.arguments[0].starts_with(&"é".repeat(65500)));
    assert!(!context.redacted);
}

#[test]
fn common_authentication_flags_and_cookies_are_not_disclosed() {
    let mut request = request();
    request.content["args"] = json!([
        "curl -u 'user:password-value' --proxy-user proxy:pass -H 'Cookie: session=private-cookie' ftp://name:password@host/file",
        "-u",
        "separate-user:separate-password",
        "--user=inline:credential",
        "-uattached-user:attached-password",
        "Cookie: first=one-private-cookie; second=two-private-cookie",
        "curl -H 'Cookie: session=three-private-cookie; csrf=four-private-cookie'"
    ]);
    let context = command_approval_context(&request).unwrap().unwrap();
    let text = serde_json::to_string(&context).unwrap();
    for secret in [
        "password-value",
        "proxy:pass",
        "private-cookie",
        "name:password",
        "separate-password",
        "inline:credential",
        "attached-password",
        "one-private-cookie",
        "two-private-cookie",
        "three-private-cookie",
        "four-private-cookie",
    ] {
        assert!(!text.contains(secret), "leaked {secret}");
    }
    assert!(context.redacted);
}

#[test]
fn complete_token68_credentials_are_redacted_without_changing_execution() {
    let token = "AZaz09-._~+/sensitive==";
    for arguments in [
        vec![format!("Bearer {token}")],
        vec![format!("bAsIc {token}")],
        vec![format!("curl --oauth2-bearer {token} https://example.test")],
        vec![format!(
            "curl --oauth2-bearer '{token}' https://example.test"
        )],
        vec!["--oauth2-bearer".into(), token.into()],
        vec![format!("--oauth2-bearer={token}")],
    ] {
        let mut request = request();
        request.content["args"] = json!(arguments);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        let released = serde_json::to_string(&context).unwrap();
        assert!(
            !released.contains("sensitive"),
            "credential suffix leaked: {released}"
        );
        assert!(context.redacted);
        assert_eq!(request, original);
    }
}

#[test]
fn absent_legacy_intent_and_non_command_disclosure() {
    let mut request = request();
    request.command_intent = None;
    assert!(command_approval_context(&request).unwrap().is_none());
    request.command_intent = Some(CommandIntent {
        justification: "A reason".into(),
    });
    request.action = "filesystem.read".into();
    assert!(command_approval_context(&request).is_err());
}

#[test]
fn escaped_controls_are_distinct_from_literal_escape_sequences() {
    let mut request = request();
    request.content["args"] = json!(["line\nbreak", "line\\nbreak", "C:\\workspace"]);
    let context = command_approval_context(&request).unwrap().unwrap();
    assert_eq!(
        context.arguments,
        ["line\\nbreak", "line\\\\nbreak", "C:\\\\workspace"]
    );
    assert_ne!(context.arguments[0], context.arguments[1]);
    assert!(!context.redacted);
}

#[test]
fn intent_and_actual_command_are_part_of_the_private_request_binding() {
    let original = request();
    let bytes = crate::kernel::canonical_bytes(&original).unwrap();
    for field in ["reason", "executable", "arguments", "cwd"] {
        let mut changed = original.clone();
        match field {
            "reason" => {
                changed.command_intent.as_mut().unwrap().justification =
                    "A different purpose.".into()
            }
            "executable" => changed.resource = "/bin/other".into(),
            "arguments" => changed.content["args"] = json!(["other"]),
            "cwd" => changed.content["cwd"] = json!("/other"),
            _ => unreachable!(),
        }
        assert_ne!(crate::kernel::canonical_bytes(&changed).unwrap(), bytes);
    }
    let mut historical = serde_json::to_value(&original).unwrap();
    historical.as_object_mut().unwrap().remove("command_intent");
    assert!(
        serde_json::from_value::<EffectRequest>(historical)
            .unwrap()
            .command_intent
            .is_none()
    );
}

#[test]
fn redaction_expansion_does_not_lower_the_valid_intent_limit() {
    let mut request = request();
    request.command_intent.as_mut().unwrap().justification = "x".repeat(512);
    request.content["environment"]["TOKEN"] = json!("x");
    let context = command_approval_context(&request).unwrap().unwrap();
    assert_eq!(context.justification, "[REDACTED]");
    assert!(context.redacted);
}

#[test]
fn explanation_redaction_uses_credential_values_before_policy_replaces_them() {
    let mut request = request();
    request.content["environment"]["password"] = json!("opaque-secret-value");
    request.command_intent.as_mut().unwrap().justification =
        "Check opaque-secret-value configuration.".into();
    let context = command_approval_context(&request).unwrap().unwrap();
    assert_eq!(context.justification, "Check [REDACTED] configuration.");
    assert!(context.redacted);
    assert_eq!(
        request.content["environment"]["password"],
        "opaque-secret-value"
    );
}

#[test]
fn complete_shell_credential_words_are_masked_and_other_arguments_are_unchanged() {
    for command in [
        r#"TOKEN=top"secret" AFTER"#,
        r#"TOKEN='top'"secret" AFTER"#,
        r#"export "PASSWORD"=top"secret" AFTER"#,
        r"TOKEN=top\ secret AFTER",
        r#"curl --password=top"secret" AFTER"#,
        r#"curl --oauth2-bearer top"secret" AFTER"#,
        r#"Bearer top"secret" AFTER"#,
        r#"TOKEN=$(printf '%s' "secret") AFTER"#,
        r#"TOKEN="$(printf '%s' "secret")" AFTER"#,
        r#"TOKEN=${VALUE:-"top secret"} AFTER"#,
        "TOKEN=`printf secret` AFTER",
    ] {
        let mut request = request();
        request.content["args"] = json!(["-c", command]);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        assert!(!context.arguments[1].contains("secret"), "{command}");
        assert!(context.arguments[1].ends_with(" AFTER"), "{command}");
        assert!(context.redacted);
        assert_eq!(request, original);
    }
}

#[test]
fn known_secrets_cannot_disable_other_credential_recognizers() {
    let mut request = request();
    request.content["environment"]["password"] = json!("pass");
    request.content["args"] = json!(["password=other-credential-tail AFTER"]);
    let context = command_approval_context(&request).unwrap().unwrap();
    assert!(!context.arguments[0].contains("other-credential-tail"));
    assert!(!context.arguments[0].contains("pass"));
    assert!(context.arguments[0].ends_with(" AFTER"));
}

#[test]
fn ambiguous_credential_words_do_not_release_a_tail() {
    for command in [
        "TOKEN='unclosed secret tail",
        "TOKEN=$(unclosed secret tail",
    ] {
        let mut request = request();
        request.content["args"] = json!([command]);
        let context = command_approval_context(&request).unwrap().unwrap();
        assert_eq!(context.arguments[0], "TOKEN=[REDACTED]");
    }
}

#[test]
fn quoted_and_escaped_literal_credential_names_do_not_release_values() {
    for command in [
        r#"curl --oauth2-"bearer" topsecret AFTER"#,
        r#"curl --"password"='topsecret' AFTER"#,
        r#"curl --pass""word topsecret AFTER"#,
        r"curl --pass\word topsecret AFTER",
        "curl --pass\\\nword topsecret AFTER",
        r#"curl -"u" user:topsecret AFTER"#,
        r#"PASS'WORD'=topsecret AFTER"#,
        r#""PASS"WORD=topsecret AFTER"#,
        r#"TO"KEN"=top"secret" AFTER"#,
        r"curl --pass^word topsecret AFTER",
    ] {
        let mut request = request();
        request.content["args"] = json!(["-c", command]);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        assert!(!context.arguments[1].contains("secret"), "{command}");
        assert!(context.arguments[1].ends_with(" AFTER"), "{command}");
        assert!(context.redacted);
        assert_eq!(request, original);
    }
    for command in ["--password '' AFTER", "TOKEN='' AFTER"] {
        let (display, _) = sanitized(command, &[]);
        assert!(display.ends_with(" AFTER"), "{display}");
    }
}

#[test]
fn an_unclosed_private_key_masks_the_remainder_without_changing_execution() {
    for command in [
        "printf '-----BEGIN PRIVATE KEY-----\nprivate-key-material",
        "printf '-----BEGIN RSA PRIVATE KEY-----\nprivate-key-material\n'",
        "printf '-----BEGIN PRI'\"VATE KEY-----\" private-key-material",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nprivate-key-material\n-----END OPENSSH PRIVATE KEY----- AFTER",
    ] {
        let mut request = request();
        request.content["args"] = json!([command]);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        assert!(!context.arguments[0].contains("private-key-material"));
        assert!(context.redacted);
        assert_eq!(request, original);
    }
    assert_eq!(
        sanitized(
            "-----BEGIN PRIVATE KEY-----\nmaterial\n-----END PRIVATE KEY----- AFTER",
            &[]
        ),
        ("[REDACTED] AFTER".into(), true)
    );
}

#[test]
fn quoted_json_credential_keys_are_not_disclosed() {
    let mut request = request();
    request.content["args"] = json!([r#"curl -d '{"password":"quoted-credential-tail"}'"#]);
    let context = command_approval_context(&request).unwrap().unwrap();
    assert!(!context.arguments[0].contains("quoted-credential-tail"));
    assert!(context.redacted);
}

#[test]
fn policy_receives_only_an_intent_digest_while_approval_binding_keeps_the_original() {
    let mut request = request();
    request.content["environment"] = json!({"password": "unique-secret-value"});
    request.command_intent.as_mut().unwrap().justification = "Check unique-secret-value.".into();
    let kernel = crate::SafetyKernel::new([]);
    let prepared = kernel.prepare(&request).unwrap();
    let before = crate::kernel::canonical_bytes(&prepared).unwrap();
    let policy = kernel.policy_projection(&prepared).unwrap();
    let justification = &policy.command_intent.as_ref().unwrap().justification;
    assert_eq!(
        justification,
        &format!(
            "sha256:{}",
            crate::kernel::sha256_hex(b"Check unique-secret-value.")
        )
    );
    assert!(!justification.contains("unique-secret-value"));
    assert!(
        !serde_json::to_string(&policy)
            .unwrap()
            .contains("unique-secret-value")
    );
    assert_eq!(crate::kernel::canonical_bytes(&prepared).unwrap(), before);
    assert_eq!(prepared.command_intent, request.command_intent);
    assert_eq!(
        command_approval_context(&request)
            .unwrap()
            .unwrap()
            .justification,
        "Check [REDACTED]."
    );
}
