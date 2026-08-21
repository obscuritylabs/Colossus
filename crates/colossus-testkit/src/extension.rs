use super::*;

/// Shared integration, pack, trust, bounds, and reconstruction checks for extension adapters.
pub fn assert_extension_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn ExtensionRepository>,
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
            .expect("missing integration")
            .is_none()
    );
    repository
        .save_integration(connection.clone(), conformance_actor("extension-user"))
        .expect("save integration");
    connection.description = "Updated connection.".into();
    connection.updated_at = "2026-07-11T12:01:00Z".into();
    repository
        .save_integration(connection.clone(), conformance_actor("extension-user"))
        .expect("update integration");
    let mut changed_identity = connection.clone();
    changed_identity.connected_at = "2026-07-12T00:00:00Z".into();
    assert!(
        repository
            .save_integration(changed_identity, conformance_actor("extension-user"))
            .is_err(),
        "connected_at must be immutable"
    );
    let disconnected = repository
        .disconnect_integration(
            "demo",
            conformance_actor("extension-user"),
            "2026-07-11T12:02:00Z",
        )
        .expect("disconnect integration");
    assert_eq!(disconnected.status, IntegrationStatus::Disconnected);
    connection.updated_at = "2026-07-11T12:03:00Z".into();
    repository
        .save_integration(connection.clone(), conformance_actor("extension-user"))
        .expect("reconnect integration");
    assert!(repository.list_integrations(0).is_err());
    assert!(repository.list_integrations(1_001).is_err());

    let manifest = PackManifest {
        format_version: 1,
        name: "demo-pack".into(),
        version: "1.0.0".into(),
        description: "Conformance pack.".into(),
        publisher: "example".into(),
        license: "Apache-2.0".into(),
        homepage: String::new(),
        capabilities: Vec::new(),
        permissions: Vec::new(),
        files: Vec::new(),
        integrations: Vec::new(),
        skills: Vec::new(),
        tools: Vec::new(),
        mcp_servers: Vec::new(),
        binaries: Vec::new(),
        docker: Vec::new(),
        docs: Vec::new(),
        tests: Vec::new(),
        dependencies: Vec::new(),
        signatures: Vec::new(),
    };
    let mut installation = PackInstallation {
        manifest,
        status: PackStatus::Enabled,
        source: "conformance".into(),
        installed_path: "/tmp/colossus-conformance-pack".into(),
        manifest_sha256: "1".repeat(64),
        trust_key_id: None,
        installed_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
    };
    repository
        .install_pack(installation.clone(), conformance_actor("extension-user"))
        .expect("install pack");
    assert!(
        repository
            .install_pack(installation.clone(), conformance_actor("extension-user"))
            .is_err(),
        "installed pack cannot be overwritten"
    );
    assert_eq!(
        repository
            .set_pack_status(
                "demo-pack",
                PackStatus::Disabled,
                conformance_actor("extension-user"),
                "2026-07-11T12:01:00Z",
            )
            .expect("disable pack")
            .status,
        PackStatus::Disabled
    );
    repository
        .set_pack_status(
            "demo-pack",
            PackStatus::Uninstalled,
            conformance_actor("extension-user"),
            "2026-07-11T12:02:00Z",
        )
        .expect("uninstall pack");
    assert!(
        repository
            .set_pack_status(
                "demo-pack",
                PackStatus::Enabled,
                conformance_actor("extension-user"),
                "2026-07-11T12:03:00Z",
            )
            .is_err(),
        "uninstalled pack cannot transition without reinstall"
    );
    installation.updated_at = "2026-07-11T12:04:00Z".into();
    repository
        .install_pack(installation.clone(), conformance_actor("extension-user"))
        .expect("reinstall pack");
    let mut batch_pack = installation.clone();
    batch_pack.manifest.name = "batch-pack".into();
    repository
        .install_packs(
            vec![batch_pack.clone()],
            conformance_actor("extension-user"),
        )
        .expect("install pack batch");
    assert!(
        repository
            .install_packs(Vec::new(), conformance_actor("extension-user"))
            .is_err()
    );
    assert!(
        repository
            .install_packs(
                vec![batch_pack.clone(), batch_pack],
                conformance_actor("extension-user"),
            )
            .is_err()
    );
    assert!(repository.list_packs(0).is_err());
    assert!(repository.list_packs(1_001).is_err());

    let trust = PublisherTrust {
        publisher: "example".into(),
        key_id: "2".repeat(64),
        public_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
        added_at: "2026-07-11T12:00:00Z".into(),
    };
    repository
        .add_publisher_trust(trust.clone(), conformance_actor("extension-user"))
        .expect("add publisher trust");
    assert!(
        repository
            .add_publisher_trust(trust.clone(), conformance_actor("extension-user"))
            .is_err(),
        "publisher/key trust binding is immutable"
    );
    assert!(repository.list_publisher_trust(0).is_err());
    assert!(repository.list_publisher_trust(1_001).is_err());
    drop(repository);

    let reopened = factory();
    assert_eq!(
        reopened
            .get_integration("demo")
            .expect("reopened integration"),
        Some(connection)
    );
    assert_eq!(
        reopened.list_integrations(10).expect("integrations").len(),
        1
    );
    assert!(reopened.get("demo").expect("aggregate get").is_some());
    assert_eq!(reopened.list(10).expect("aggregate list").len(), 1);
    assert_eq!(
        reopened.get_pack("demo-pack").expect("reopened pack"),
        Some(installation)
    );
    assert_eq!(reopened.list_packs(10).expect("packs").len(), 2);
    assert_eq!(
        reopened
            .get_publisher_trust(&trust.publisher, &trust.key_id)
            .expect("publisher trust"),
        Some(trust)
    );
    assert_eq!(
        reopened
            .list_publisher_trust(10)
            .expect("publisher trust list")
            .len(),
        1
    );
}
