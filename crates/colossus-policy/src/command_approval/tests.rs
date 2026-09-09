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
fn dynamically_assembled_words_and_values_are_never_evaluated_or_released() {
    for arguments in [
        vec!["-c", "curl --oauth2-$(printf bearer) private-value END"],
        vec!["-c", "curl --oauth2-`printf bearer` 'private-value' END"],
        vec!["-c", "curl --oauth2-${FIELD} private-value END"],
        vec!["-c", "curl --oauth2-%FIELD% private-value END"],
        vec!["-c", "curl --oauth2-!FIELD! private-value END"],
        vec!["-c", "curl ${FLAG} private-value END"],
        vec![
            "-Command",
            "curl ('--oauth2-' + 'bearer') private-value END",
        ],
        vec!["-c", "curl --{oauth2-bearer,user} private-value END"],
        vec!["-c", "curl --oauth2-bear?r private-value END"],
        vec!["${FLAG}", "private-value", "END"],
        vec!["--oauth2-$(printf bearer)", "private-value", "END"],
    ] {
        let mut request = request();
        request.content["args"] = json!(arguments);
        // Exact-value masking must not disable dynamic field-name recognition.
        request.content["environment"]["TOKEN"] = json!("$");
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        let released = serde_json::to_string(&context).unwrap();
        assert!(!released.contains("private-value"), "leaked: {released}");
        assert!(released.contains("END"));
        assert!(context.redacted);
        assert_eq!(request, original);
    }
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
        sanitized("ééé END", &["é", "éé"], &[]),
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
fn ambiguous_short_password_options_are_recognized_in_shell_and_prepared_argv() {
    for (program, prefix, option, attached) in [
        ("ssh-keygen", "", "-N", true),
        ("ssh-keygen", "", "-P", true),
        ("ssh-keygen", "", "-qN", true),
        ("sshpass", "", "-p", true),
        ("mysql", "", "-p", true),
        ("mariadb", "", "-p", true),
        ("mongosh", "", "-p", true),
        ("7z", "a", "-p", true),
        ("redis-cli", "", "-a", true),
        ("security", "add-generic-password", "-w", true),
        ("openssl", "enc", "-k", false),
        ("openssl", "enc", "-K", false),
        ("openssl", "cms", "-pwri_password", false),
        ("openssl", "cms", "-secretkey", false),
        ("openssl", "pkcs12", "-password", false),
        ("openssl", "dgst", "-hmac", false),
        ("openssl", "mac", "-macopt", false),
        ("keytool", "-genkeypair", "-storepass", false),
        ("keytool", "-genkeypair", "-keypass", false),
        ("keytool", "-importkeystore", "-srcstorepass", false),
        ("keytool", "-importkeystore", "-deststorepass", false),
        ("keytool", "-importkeystore", "-srckeypass", false),
        ("keytool", "-importkeystore", "-destkeypass", false),
        ("keytool", "-storepasswd", "-new", false),
        ("keytool", "-keypasswd", "-KEYPASS", false),
        ("jarsigner", "", "-storepass", false),
        ("jarsigner", "", "-keypass", false),
        ("docker", "login", "-p", true),
        ("podman", "login", "-p", true),
    ] {
        for join in [" ", "=", ""] {
            if join.is_empty() && !attached {
                continue;
            }
            let mut arguments = Vec::new();
            if !prefix.is_empty() {
                arguments.push(prefix.to_owned());
            }
            if join == " " {
                arguments.extend([option.into(), "private-value private-tail".into()]);
            } else {
                arguments.push(format!("{option}{join}private-value private-tail"));
            }
            arguments.push("PUBLIC_END".into());
            for shell in [false, true] {
                let mut request = request();
                if shell {
                    request.content["args"] = json!([
                        "-c",
                        format!(
                            "/usr/bin/{program} {prefix} {option}{join}'private-value private-tail' PUBLIC_END"
                        )
                    ]);
                } else {
                    request.resource = format!("/usr/bin/{program}");
                    request.content["args"] = json!(arguments);
                }
                let original = request.clone();
                let context = command_approval_context(&request).unwrap().unwrap();
                let released = serde_json::to_string(&context).unwrap();
                assert!(!released.contains("private-value"), "{released}");
                assert!(!released.contains("private-tail"), "{released}");
                assert!(released.contains("PUBLIC_END"), "{released}");
                assert!(context.redacted);
                assert_eq!(request, original);
            }
        }
    }
    for command in [
        r#""C:\Tools\ssh-keygen.exe" -N 'private-value' PUBLIC_END"#,
        "ssh-\"keygen\" -P 'private-value' PUBLIC_END",
        "sudo keytool '-storepass' 'private-value' PUBLIC_END",
        "env openssl cms -pwri_\"password\" 'private-value' PUBLIC_END",
        r#""C:\Tools\keytool.exe" -keypass 'private-value' PUBLIC_END"#,
    ] {
        let (display, redacted) = sanitized(command, &[], &[]);
        assert!(!display.contains("private-value"), "{display}");
        assert!(display.ends_with("PUBLIC_END"));
        assert!(redacted);
    }
}

#[test]
fn ordinary_short_options_remain_reviewable_without_a_credential_command_hint() {
    for (program, arguments) in [
        ("cargo", vec!["test", "-p", "colossus-runtime"]),
        ("cargo", vec!["test", "-p", "mysql"]),
        ("curl", vec!["-N", "https://example.test"]),
        ("docker", vec!["run", "-p", "8080:80", "image"]),
        ("openssl", vec!["s_client", "-key", "key.pem"]),
        ("openssl", vec!["req", "-new", "-key", "key.pem"]),
        ("keytool", vec!["-storepasswd", "-keystore", "store.jks"]),
        ("keytool", vec!["-keypasswd", "-keystore", "store.jks"]),
        ("jarsigner", vec!["-keystore", "store.jks", "public.jar"]),
    ] {
        for shell in [false, true] {
            let mut request = request();
            if shell {
                request.content["args"] =
                    json!(["-c", format!("{program} {}", arguments.join(" "))]);
            } else {
                request.resource = format!("/usr/bin/{program}");
                request.content["args"] = json!(arguments);
            }
            let context = command_approval_context(&request).unwrap().unwrap();
            assert_eq!(json!(context.arguments), request.content["args"]);
            assert!(!context.redacted);
        }
    }
}

#[test]
fn literal_program_hints_survive_wrapped_argument_vectors() {
    for (executable, arguments) in [
        (
            "/usr/bin/env",
            vec![
                "MODE=test",
                "ssh-keygen",
                "-N",
                "private-value",
                "PUBLIC_END",
            ],
        ),
        (
            "/usr/bin/sudo",
            vec![
                "-u",
                "root",
                "/usr/bin/ssh-keygen",
                "-Pprivate-value",
                "PUBLIC_END",
            ],
        ),
        (
            "/usr/bin/timeout",
            vec!["10", "sshpass", "-p", "private-value", "PUBLIC_END"],
        ),
        (
            r"C:\Windows\System32\cmd.exe",
            vec![
                "/C",
                r"C:\Tools\SSH-KEYGEN.EXE",
                "-N",
                "private-value",
                "PUBLIC_END",
            ],
        ),
    ] {
        let mut request = request();
        request.resource = executable.into();
        request.content["args"] = json!(arguments);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        let released = serde_json::to_string(&context).unwrap();
        assert!(!released.contains("private-value"), "{released}");
        assert!(released.contains("PUBLIC_END"));
        assert!(context.redacted);
        assert_eq!(request, original);
    }
}

#[test]
fn command_hints_do_not_mask_options_in_a_later_independent_command() {
    for command in [
        "mysql --version; cargo test -p mysql",
        "ssh-keygen -V | curl -N https://example.test",
        "docker login --help && docker run -p 8080:80 image",
    ] {
        let (display, redacted) = sanitized(command, &[], &[]);
        assert_eq!(display, command);
        assert!(!redacted);
    }
    let command = format!("{}-p private-value PUBLIC_END", "mysql ".repeat(10_000));
    let (display, redacted) = sanitized(&command, &[], &[]);
    assert!(!display.contains("private-value"));
    assert!(display.ends_with("PUBLIC_END"));
    assert!(redacted);
}

#[test]
fn assignments_inside_quoted_words_never_release_a_private_tail() {
    for command in [
        r#"export "TOKEN=private-value private-tail"; echo PUBLIC_END"#,
        r#"export 'PASSWORD=private-value;private-tail'; echo PUBLIC_END"#,
        r#"printf '%s' "TOKEN=private-value&private-tail" PUBLIC_END"#,
        r#"ssh-keygen '-Nprivate-value private-tail' -f PUBLIC_END"#,
    ] {
        let (display, redacted) = sanitized(command, &[], &[]);
        assert!(!display.contains("private-value"), "{display}");
        assert!(!display.contains("private-tail"), "{display}");
        assert!(display.contains("PUBLIC_END"), "{display}");
        assert!(redacted);
    }
}

#[test]
fn passphrase_and_certificate_options_share_shell_and_argv_redaction() {
    for option in CREDENTIAL_VALUE_OPTIONS
        .iter()
        .copied()
        .chain(["--passphrase", "-E", "-svE"])
    {
        for arguments in [
            vec![
                option.to_owned(),
                "client.pem:private-value private-tail".into(),
                "PUBLIC_END".into(),
            ],
            vec![
                format!("{option}=client.pem:private-value private-tail"),
                "PUBLIC_END".into(),
            ],
            vec![
                "-c".into(),
                format!("curl {option} 'client.pem:private-value private-tail' PUBLIC_END"),
            ],
            vec![
                "-c".into(),
                format!("curl {option}='client.pem:private-value private-tail' PUBLIC_END"),
            ],
        ] {
            let mut request = request();
            request.content["args"] = json!(arguments);
            let original = request.clone();
            let context = command_approval_context(&request).unwrap().unwrap();
            let released = serde_json::to_string(&context).unwrap();
            assert!(!released.contains("private-value"), "{released}");
            assert!(!released.contains("private-tail"), "{released}");
            assert!(released.contains("PUBLIC_END"), "{released}");
            assert!(context.redacted);
            assert_eq!(request, original);
        }
    }
    for arguments in [
        vec!["-Eclient.pem:private-value", "PUBLIC_END"],
        vec!["-c", "curl -svE'client.pem:private-value' PUBLIC_END"],
        vec!["-c", "curl --pa\"ss\" 'private-value' PUBLIC_END"],
        vec![
            "-c",
            "curl https://example.test/?pass=private-value PUBLIC_END",
        ],
    ] {
        let mut request = request();
        request.content["args"] = json!(arguments);
        let context = command_approval_context(&request).unwrap().unwrap();
        let released = serde_json::to_string(&context).unwrap();
        assert!(!released.contains("private-value"), "{released}");
        assert!(released.contains("PUBLIC_END"));
        assert!(context.redacted);
    }
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
fn short_and_long_header_payloads_are_contained_without_evaluating_quotes() {
    for arguments in [
        vec!["-HAuthorization: Digest private-value"],
        vec!["-HProxy-Authorization: Digest private-value"],
        vec!["-HCookie: session=private-value; other=private-tail"],
        vec!["-H", "Authorization: Digest private-value"],
        vec!["-svH", "Authorization: Digest private-value"],
        vec!["-svHAuthorization: Digest private-value"],
        vec!["--header", "Authorization: Digest private-value"],
        vec!["--header=Authorization: Digest private-value"],
        vec!["-c", "curl \"-HAuthorization: Digest private-value\" END"],
        vec!["-c", "curl -H'Author''ization: Digest private-value' END"],
        vec!["-c", "curl -svH'Author''ization: Digest private-value' END"],
        vec![
            "-c",
            "curl --header 'Author''ization: Digest private-value' END",
        ],
    ] {
        let mut request = request();
        request.content["args"] = json!(arguments);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        let released = serde_json::to_string(&context).unwrap();
        assert!(!released.contains("private-value"), "{released}");
        assert!(!released.contains("private-tail"), "{released}");
        assert!(context.redacted);
        assert_eq!(request, original);
    }
}

#[test]
fn cookie_short_options_redact_separated_attached_and_quoted_values() {
    for arguments in [
        vec!["-b", "session=private-value; other=private-tail", "END"],
        vec!["-sb", "session=private-value; other=private-tail", "END"],
        vec!["-svbsession=private-value", "END"],
        vec!["-bsession=private-value", "END"],
        vec!["-b=session=private-value", "END"],
        vec!["--cookie", "session=private-value", "END"],
        vec!["-c", "curl -b session=private-value END"],
        vec!["-c", "curl -sb session=private-value END"],
        vec![
            "-c",
            "curl -#b'session=private-value; other=private-tail' END",
        ],
        vec![
            "-c",
            "curl -b'session=private-value; other=private-tail' END",
        ],
        vec!["-c", "curl -\"b\" session=private-value END"],
        vec!["-c", "curl --cookie='session=private-value' END"],
    ] {
        let mut request = request();
        request.content["args"] = json!(arguments);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        let released = serde_json::to_string(&context).unwrap();
        assert!(!released.contains("private-value"), "{released}");
        assert!(!released.contains("private-tail"), "{released}");
        assert!(released.contains("END"));
        assert!(context.redacted);
        assert_eq!(request, original);
    }
}

#[test]
fn attached_argv_values_use_argument_not_shell_boundaries() {
    for prefix in ["-sb", "-H", "-su", "--cookie=", "--header=", "--password="] {
        let mut request = request();
        request.content["args"] = json!([
            format!("{prefix}session=private-value; other=private-tail \"quoted\"\nEND_PRIVATE"),
            "PUBLIC_END"
        ]);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        assert_eq!(
            context.arguments,
            [format!("{prefix}[REDACTED]"), "PUBLIC_END".into()]
        );
        assert!(context.redacted);
        assert_eq!(request, original);
    }
    for empty in ["--password=", "--cookie=", "--header="] {
        let mut request = request();
        request.content["args"] = json!([empty, "PUBLIC_END"]);
        let context = command_approval_context(&request).unwrap().unwrap();
        assert_eq!(context.arguments, [empty, "PUBLIC_END"]);
        assert!(!context.redacted);
    }
}

#[test]
fn grouped_short_options_stop_at_the_first_credential_value_boundary() {
    for option in ["-su", "-sU", "-sb", "-svH"] {
        let mut request = request();
        request.content["args"] = json!([option, "private-value", "END"]);
        let context = command_approval_context(&request).unwrap().unwrap();
        assert_eq!(context.arguments, [option, "[REDACTED]", "END"]);

        // The b/u letters in an attached value are not additional options;
        // neither an attached-value tail nor the following public arg is lost.
        request.content["args"] = json!([format!("{option}privatebuvalue"), "END"]);
        let original = request.clone();
        let context = command_approval_context(&request).unwrap().unwrap();
        assert_eq!(
            context.arguments,
            [format!("{option}[REDACTED]"), "END".into()]
        );
        assert!(context.redacted);
        assert_eq!(request, original);
    }
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
        let (display, _) = sanitized(command, &[], &[]);
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
            &[],
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
