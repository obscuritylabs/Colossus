use super::*;
use serde_json::{Value, json};

#[test]
fn host_credential_headers_preserve_only_valid_reference_shapes() {
    for reference in [
        "host:mcp-github-token",
        "host:GitHub_1.token",
        &format!("host:{}", "a".repeat(128)),
    ] {
        for scheme in [None, Some(Value::Null), Some(json!("Bearer"))] {
            let mut header = json!({"reference": reference});
            if let Some(scheme) = scheme {
                header["scheme"] = scheme;
            }
            let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
            request.content["credential_headers"] = json!({"Authorization": header});
            let prepared = SafetyKernel::new(["mcp.invoke".into()])
                .prepare(&request)
                .expect("prepare");
            assert_eq!(
                prepared.content["credential_headers"]["Authorization"],
                header
            );
        }
    }
}

#[test]
fn malformed_host_headers_and_nested_lookalikes_remain_redacted() {
    let mut invalid = vec![
        json!({"reference": "host:"}),
        json!({"reference": "host:bad/name"}),
        json!({"reference": "host:bad name"}),
        json!({"reference": "host:bad\nname"}),
        json!({"reference": "host:é"}),
        json!({"reference": format!("host:{}", "a".repeat(129))}),
        json!({"reference": "must-not-leak"}),
        json!({"reference": 42}),
        json!({"scheme": "Bearer"}),
        json!({"reference": "host:valid", "scheme": "Bearer must-not-leak"}),
        json!({"reference": "host:valid", "scheme": ""}),
        json!({"reference": "host:valid", "scheme": "a".repeat(65)}),
        json!({"reference": "host:valid", "scheme": true}),
        json!({"reference": "host:valid", "value": "must-not-leak"}),
    ];
    invalid.push(json!("Bearer must-not-leak"));
    for header in invalid {
        let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
        request.content["credential_headers"] = json!({"Authorization": header});
        request.content["operation"]["arguments"] = json!({
            "credential_headers": {"Authorization": {"reference": "host:valid", "scheme": "Bearer"}},
            "password": "host:valid"
        });
        let prepared = SafetyKernel::new(["mcp.invoke".into()])
            .prepare(&request)
            .expect("prepare");
        assert_eq!(
            prepared.content["credential_headers"]["Authorization"]["redacted"],
            true
        );
        assert_eq!(
            prepared.content["operation"]["arguments"]["credential_headers"]["Authorization"]["redacted"],
            true
        );
        assert_eq!(
            prepared.content["operation"]["arguments"]["password"]["redacted"],
            true
        );
        assert!(!prepared.content.to_string().contains("must-not-leak"));
    }
}

#[test]
fn mcp_schema_property_names_survive_preparation_while_argument_secrets_are_redacted() {
    let schema = json!({
        "type": "object",
        "properties": {
            "apiKey": {"type": "string"},
            "authorization": {"anyOf": [{"type": "string"}, {"type": "null"}]},
            "nested": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"password": {"$ref": "#/$defs/password"}}
                }
            }
        },
        "$defs": {"password": {"type": "string", "minLength": 1}},
        "dependentRequired": {"apiKey": ["authorization"]},
        "dependentSchemas": {"apiKey": {"required": ["authorization"]}},
        "dependencies": {"password": ["apiKey"]},
        "additionalProperties": false
    });
    for action in ["mcp.call", "plugin.mcp.example.server.call"] {
        let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
        request.action = action.into();
        request.content["operation"]["input_schema"] = schema.clone();
        request.content["operation"]["schema_sha256"] = json!(crate::sha256_hex(
            &crate::canonical_bytes(&schema).expect("schema bytes")
        ));
        request.content["operation"]["arguments"] = json!({
            "apiKey": "must-not-leak",
            "authorization": "env:AUTHORIZATION",
            "nested": [{"password": "must-not-leak"}]
        });
        let prepared = SafetyKernel::new(["mcp.invoke".into()])
            .prepare(&request)
            .expect("prepare MCP request");
        assert_eq!(prepared.content["operation"]["input_schema"], schema);
        assert_eq!(
            prepared.content["operation"]["schema_sha256"],
            request.content["operation"]["schema_sha256"]
        );
        assert_eq!(
            prepared.content["operation"]["arguments"]["apiKey"]["redacted"],
            true
        );
        assert_eq!(
            prepared.content["operation"]["arguments"]["authorization"],
            "env:AUTHORIZATION"
        );
        assert!(!prepared.content.to_string().contains("must-not-leak"));
    }
}

#[test]
fn provider_tool_schemas_preserve_sensitive_property_names() {
    let schema = json!({
        "type": "object",
        "properties": {
            "api_key": {"type": "string"},
            "password": {"type": "string", "default": null}
        },
        "additionalProperties": false
    });
    for action in [
        "provider.echo",
        "provider.openai.responses",
        "provider.openai.codex",
        "provider.openai.chat",
    ] {
        let request = effect_request(
            system_actor("schema-test"),
            action,
            "provider:test",
            json!({
                "request": {
                    "tools": [
                        {"name": "first", "input_schema": schema},
                        {"name": "second", "input_schema": schema}
                    ],
                    "messages": [{"input_schema": {"api_key": "must-not-leak"}}]
                }
            }),
        );
        let prepared = SafetyKernel::new([])
            .prepare(&request)
            .expect("provider schema");
        for tool in prepared.content["request"]["tools"]
            .as_array()
            .expect("tools")
        {
            assert_eq!(tool["input_schema"], schema);
        }
        assert!(!prepared.content.to_string().contains("must-not-leak"));
    }
}

#[test]
fn schema_exemption_does_not_apply_to_arguments_or_unrelated_effects() {
    let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
    let data = json!({
        "type": "object",
        "properties": {"api_key": {"type": "string", "default": "must-not-leak"}}
    });
    request.content["operation"]["arguments"] = json!({"input_schema": data});
    request.content["input_schema"] = data.clone();
    let kernel = SafetyKernel::new(["mcp.invoke".into()]);
    let prepared = kernel.prepare(&request).expect("redacted arguments");
    assert!(!prepared.content.to_string().contains("must-not-leak"));
    assert_eq!(
        prepared.content["operation"]["arguments"]["input_schema"]["properties"]["api_key"]["redacted"],
        true
    );
    request.content["operation"]["input_schema"] = data;
    for action in [
        "mcp.tools",
        "network.http",
        "plugin.mcp.example.server.tools",
    ] {
        request.action = action.into();
        let prepared = kernel.prepare(&request).expect("ordinary effect redaction");
        assert!(!prepared.content.to_string().contains("must-not-leak"));
        assert_eq!(
            prepared.content["operation"]["input_schema"]["properties"]["api_key"]["redacted"],
            true
        );
    }
}

#[test]
fn schemas_with_literal_secrets_fail_without_disclosing_or_mutating_them() {
    let mut schemas = vec![
        json!({"type": "object", "properties": {"apiKey": "must-not-leak"}}),
        json!({"type": "object", "examples": [{"apiKey": "must-not-leak"}]}),
        json!({"type": "object", "default": {"apiKey": "must-not-leak"}}),
        json!({"type": "object", "apiKey": "must-not-leak"}),
        json!({
            "type": "object",
            "properties": {"apiKey": {"$ref": "#/$defs/credential"}},
            "$defs": {"credential": {"type": "string", "default": "must-not-leak"}}
        }),
        json!({
            "type": "object",
            "properties": {"apiKey": {"$ref": "#/$defs/first"}},
            "$defs": {
                "first": {"$ref": "#/$defs/second"},
                "second": {"$ref": "#/$defs/first", "enum": ["must-not-leak"]}
            }
        }),
        json!({
            "type": "object",
            "properties": {"apiKey": {"$ref": "#/$defs/embedded/$defs/alias"}},
            "$defs": {
                "credential": {"type": "string"},
                "embedded": {
                    "$id": "https://example.test/embedded",
                    "$defs": {
                        "alias": {"$ref": "#/$defs/credential"},
                        "credential": {"const": "must-not-leak"}
                    }
                }
            }
        }),
    ];
    for keyword in ["default", "examples", "enum", "const"] {
        let literal = if matches!(keyword, "examples" | "enum") {
            json!(["must-not-leak"])
        } else {
            json!("must-not-leak")
        };
        schemas.push(json!({
            "type": "object",
            "properties": {"apiKey": {"type": "string", keyword: literal}}
        }));
        schemas.push(json!({
            "type": "object",
            "properties": {"apiKey": {"anyOf": [{"type": "string", keyword: literal}]}}
        }));
    }
    for schema in schemas {
        let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
        request.content["operation"]["input_schema"] = schema.clone();
        let error = SafetyKernel::new(["mcp.invoke".into()])
            .prepare(&request)
            .expect_err("secret-bearing schema");
        assert!(matches!(error, GatewayError::Safety(_)));
        assert!(!error.to_string().contains("must-not-leak"));
        assert_eq!(request.content["operation"]["input_schema"], schema);
    }
}

#[test]
fn sensitive_schema_references_preserve_safe_cycles_and_reject_uninspectable_targets() {
    let safe = json!({
        "type": "object",
        "properties": {"apiKey": {"$ref": "#/$defs/credential"}},
        "$defs": {"credential": {"anyOf": [
            {"type": "string"},
            {"type": "array", "items": {"$ref": "#/$defs/credential"}}
        ]}}
    });
    let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
    request.content["operation"]["input_schema"] = safe.clone();
    let kernel = SafetyKernel::new(["mcp.invoke".into()]);
    assert_eq!(
        kernel.prepare(&request).expect("safe cycle").content["operation"]["input_schema"],
        safe
    );
    for reference in [
        "#/$defs/missing",
        "#credential",
        "https://example.test/schema.json",
    ] {
        request.content["operation"]["input_schema"]["properties"]["apiKey"]["$ref"] =
            json!(reference);
        assert!(matches!(
            kernel.prepare(&request),
            Err(GatewayError::Safety(_))
        ));
    }

    // Reusing the same definition across many branches must not recursively
    // expand an exponential number of reference paths during preparation.
    let mut shared = json!({"properties": {"apiKey": {"$ref": "#/$defs/level0"}}, "$defs": {}});
    for level in 0..2048 {
        let next = format!("#/$defs/level{}", level + 1);
        shared["$defs"][format!("level{level}")] =
            json!({"allOf": [{"$ref": next}, {"$ref": next}]});
    }
    shared["$defs"]["level2048"] = json!({"type": "string"});
    request.content["operation"]["input_schema"] = shared.clone();
    assert_eq!(
        kernel
            .prepare(&request)
            .expect("shared definitions")
            .content["operation"]["input_schema"],
        shared
    );
}

#[test]
fn schema_reference_scope_uses_the_declared_dialect() {
    for (dialect, identifier, definitions) in [
        (
            "https://json-schema.org/draft/2020-12/schema",
            "id",
            "$defs",
        ),
        (
            "https://json-schema.org/draft/2019-09/schema",
            "id",
            "$defs",
        ),
        (
            "http://json-schema.org/draft-07/schema#",
            "id",
            "definitions",
        ),
        (
            "http://json-schema.org/draft-04/schema#",
            "$id",
            "definitions",
        ),
    ] {
        let schema = json!({
            "$schema": dialect,
            "properties": {"apiKey": {"$ref": format!("#/{definitions}/wrapper")}},
            definitions: {
                "credential": {"default": "must-not-leak"},
                "wrapper": {
                    identifier: "https://example.test/ignored",
                    "allOf": [{"$ref": format!("#/{definitions}/credential")}],
                    definitions: {"credential": {"type": "string"}}
                }
            }
        });
        let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
        request.content["operation"]["input_schema"] = schema;
        assert!(
            matches!(
                SafetyKernel::new(["mcp.invoke".into()]).prepare(&request),
                Err(GatewayError::Safety(_))
            ),
            "ignored identifier in {dialect}"
        );
        let actual_identifier = if identifier == "id" { "$id" } else { "id" };
        let wrapper = &mut request.content["operation"]["input_schema"][definitions]["wrapper"];
        wrapper.as_object_mut().expect("wrapper").remove(identifier);
        wrapper[actual_identifier] = json!("https://example.test/embedded");
        SafetyKernel::new(["mcp.invoke".into()])
            .prepare(&request)
            .expect("dialect-specific embedded resource");

        if definitions == "definitions" {
            request.content["operation"]["input_schema"][definitions]["wrapper"]
                [actual_identifier] = json!("#wrapper");
            assert!(
                matches!(
                    SafetyKernel::new(["mcp.invoke".into()]).prepare(&request),
                    Err(GatewayError::Safety(_))
                ),
                "legacy anchor does not change resource"
            );
            let wrapper = &mut request.content["operation"]["input_schema"][definitions]["wrapper"];
            wrapper[actual_identifier] = json!("https://example.test/ignored-ref-sibling");
            wrapper["$ref"] = json!(format!("#/{definitions}/credential"));
            assert!(
                matches!(
                    SafetyKernel::new(["mcp.invoke".into()]).prepare(&request),
                    Err(GatewayError::Safety(_))
                ),
                "legacy $ref sibling does not change resource"
            );
        }
    }

    let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
    request.content["operation"]["input_schema"] = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "properties": {"apiKey": {"$ref": "#/$defs/embedded/definitions/alias"}},
        "definitions": {"credential": {"type": "string"}},
        "$defs": {"embedded": {
            "$schema": "http://json-schema.org/draft-04/schema#",
            "id": "https://example.test/draft4",
            "definitions": {
                "alias": {"$ref": "#/definitions/credential"},
                "credential": {"default": "must-not-leak"}
            }
        }}
    });
    let kernel = SafetyKernel::new(["mcp.invoke".into()]);
    assert!(matches!(
        kernel.prepare(&request),
        Err(GatewayError::Safety(_))
    ));
    request.content["operation"]["input_schema"]["$defs"]["embedded"]["definitions"]["credential"] =
        json!({"type": "string"});
    kernel
        .prepare(&request)
        .expect("mixed dialects without secrets");
    request.content["operation"]["input_schema"]["$schema"] =
        json!("https://example.test/unknown-dialect");
    assert!(matches!(
        kernel.prepare(&request),
        Err(GatewayError::Safety(_))
    ));
}

#[test]
fn schema_reference_fragments_decode_before_json_pointer_tokens() {
    for (name, reference) in [
        ("credential value", "#/$defs/credential%20value"),
        ("credential/value", "#%2f$defs%2fcredential%7E1value"),
        ("credential~value", "#/$defs/credential%7e0value"),
        ("credentialé", "#/$defs/credential%C3%A9"),
        ("credential%20value", "#/$defs/credential%2520value"),
    ] {
        let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
        request.content["operation"]["input_schema"] = json!({
            "properties": {"apiKey": {"$ref": reference}},
            "$defs": {name: {"type": "string"}}
        });
        let kernel = SafetyKernel::new(["mcp.invoke".into()]);
        assert_eq!(
            kernel
                .prepare(&request)
                .expect("encoded local reference")
                .content["operation"]["input_schema"],
            request.content["operation"]["input_schema"]
        );
        request.content["operation"]["input_schema"]["$defs"][name]["default"] =
            json!("must-not-leak");
        assert!(matches!(
            kernel.prepare(&request),
            Err(GatewayError::Safety(_))
        ));
    }
    for reference in [
        "#/$defs/value%",
        "#/$defs/value%2",
        "#/$defs/value%GG",
        "#/$defs/value%FF",
    ] {
        let mut request = mcp_call_request("streamable_http", "https://example.test/mcp", None);
        request.content["operation"]["input_schema"] = json!({
            "properties": {"apiKey": {"$ref": reference}},
            "$defs": {"value": {"type": "string"}}
        });
        assert!(matches!(
            SafetyKernel::new(["mcp.invoke".into()]).prepare(&request),
            Err(GatewayError::Safety(_))
        ));
    }
}
