use super::*;

/// Shared native-integration repository bounds and reconstruction checks.
pub fn assert_integration_repository_conformance<F, T>(factory: F)
where
    F: Fn() -> T,
    T: IntegrationRepository,
{
    let repository = factory();
    let operation = IntegrationOperation {
        tool: ToolSpec {
            name: "openapi.demo.read".into(),
            description: "Read a demo record.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            effect_action: Some("openapi.demo.read".into()),
            capability: Some("integration.invoke".into()),
            max_output_bytes: 1024,
        },
        operation_id: "read".into(),
        method: "GET".into(),
        path: "/records".into(),
        path_parameters: Vec::new(),
        query_parameters: Vec::new(),
        accepts_body: false,
    };
    let mut connection = IntegrationConnection {
        name: "demo".into(),
        kind: IntegrationKind::OpenApi,
        status: IntegrationStatus::Connected,
        title: "Demo".into(),
        description: "Conformance connection.".into(),
        base_url: "https://example.com".into(),
        auth: IntegrationAuth::None,
        credential_reference: None,
        scopes: Vec::new(),
        operations: vec![operation],
        manifest_sha256: "0".repeat(64),
        connected_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
    };
    assert!(
        repository
            .get_integration("demo")
            .expect("missing")
            .is_none()
    );
    repository
        .save_integration(connection.clone(), conformance_actor("integration-user"))
        .expect("save");
    connection.description = "Updated connection.".into();
    connection.updated_at = "2026-07-11T12:01:00Z".into();
    repository
        .save_integration(connection.clone(), conformance_actor("integration-user"))
        .expect("update");
    let mut changed_identity = connection.clone();
    changed_identity.connected_at = "2026-07-12T00:00:00Z".into();
    assert!(
        repository
            .save_integration(changed_identity, conformance_actor("integration-user"))
            .is_err()
    );
    repository
        .disconnect_integration(
            "demo",
            conformance_actor("integration-user"),
            "2026-07-11T12:02:00Z",
        )
        .expect("disconnect");
    connection.updated_at = "2026-07-11T12:03:00Z".into();
    repository
        .save_integration(connection.clone(), conformance_actor("integration-user"))
        .expect("reconnect");
    assert!(repository.list_integrations(0).is_err());
    assert!(repository.list_integrations(1_001).is_err());
    drop(repository);

    let reopened = factory();
    assert_eq!(
        reopened.get_integration("demo").expect("reopen"),
        Some(connection)
    );
    assert_eq!(reopened.list_integrations(10).expect("list").len(), 1);
    assert!(reopened.get("demo").expect("aggregate get").is_some());
    assert_eq!(reopened.list(10).expect("aggregate list").len(), 1);
}
