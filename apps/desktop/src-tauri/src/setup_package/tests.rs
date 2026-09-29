use super::{archive, configuration, types::*};
use crate::desktop_settings::DesktopSettings;
use serde_json::json;
use std::io::{Cursor, Write};

fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in files {
        zip.start_file(*path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
const MANIFEST: &str = "schemaVersion: 1\nid: company\nname: Company\nversion: '1'\nproviders:\n  company:\n    displayName: Company AI\n    descriptionMarkdown: 'Get a token from **your administrator**.'\n";
const CONFIG: &str = "schemaVersion: 3\nproviders:\n  profiles:\n    company:\n      kind: open_ai_compatible\n      baseUrl: https://ai.example.com/v1\n      credentialReference: env:COMPANY_TOKEN\n";

fn bytes() -> Vec<u8> {
    zip(&[
        ("manifest.yaml", MANIFEST.as_bytes()),
        ("config.yaml", CONFIG.as_bytes()),
    ])
}
fn saved() -> SavedSetupPackage {
    let source = archive::read(&bytes()).unwrap();
    let canonical = json!({"providers":{"profiles":{"company":{"kind":"open_ai_compatible","baseUrl":"https://ai.example.com/v1","credentialReference":"env:COMPANY_TOKEN"}}},"models":{"profiles":{},"roles":{}}});
    configuration::inspected(source, &canonical, &bytes()).unwrap()
}

#[test]
fn setup_package_offline_roundtrip_preserves_instructions_and_omits_credentials() {
    let mut package = saved();
    package.providers[0].connection.credential_id = Some("local-secret-handle".into());
    let exported = archive::write(&package).unwrap();
    let read = archive::read(&exported).unwrap();
    assert_eq!(
        read.manifest.providers["company"].description_markdown,
        "Get a token from **your administrator**."
    );
    assert!(!read.config_yaml.contains("local-secret-handle"));
    assert!(read.config_yaml.contains("env:COMPANY_TOKEN"));
}

#[test]
fn setup_package_rejects_traversal_unreferenced_and_case_colliding_members() {
    for extra in [
        "../escape",
        "assets/../escape",
        "/absolute",
        "C:/escape",
        "assets\\icon.png",
        "unreferenced.txt",
        "CONFIG.yaml",
    ] {
        assert!(
            archive::read(&zip(&[
                ("manifest.yaml", MANIFEST.as_bytes()),
                ("config.yaml", CONFIG.as_bytes()),
                (extra, b"data")
            ]))
            .is_err(),
            "{extra}"
        );
    }
}

#[test]
fn setup_package_rejects_unsupported_versions_and_unknown_manifest_fields() {
    for manifest in [
        MANIFEST.replace("schemaVersion: 1", "schemaVersion: 99"),
        format!("{MANIFEST}script: execute\n"),
    ] {
        assert!(
            archive::read(&zip(&[
                ("manifest.yaml", manifest.as_bytes()),
                ("config.yaml", CONFIG.as_bytes())
            ]))
            .is_err()
        );
    }
}

#[test]
fn setup_package_validation_is_provider_only_and_does_not_grant_runtime_access() {
    let source = archive::read(&bytes()).unwrap();
    let inspection: serde_json::Value =
        serde_json::from_str(&configuration::inspection_yaml(&source).unwrap()).unwrap();
    assert_eq!(
        inspection["sandbox"]["networkDestinations"],
        json!(["https://ai.example.com"])
    );
    assert_eq!(
        inspection["models"]["roles"]["primary"],
        "__setup_validation"
    );
    assert!(!source.config_yaml.contains("sandbox"));
    for suffix in [
        "access:\n  profile: allow_all\n",
        "mcp:\n  servers: {}\n",
        "storage:\n  path: /secret\n",
    ] {
        let mut source = archive::read(&bytes()).unwrap();
        source.config_yaml.push_str(suffix);
        assert!(configuration::inspection_yaml(&source).is_err());
    }
}

#[test]
fn setup_package_rejects_machine_credentials_and_endpoint_secrets() {
    for (old, new) in [
        ("env:COMPANY_TOKEN", "host:other-machine"),
        ("env:COMPANY_TOKEN", "literal-key"),
        (
            "https://ai.example.com/v1",
            "https://user:password@ai.example.com/v1",
        ),
    ] {
        let mut source = archive::read(&bytes()).unwrap();
        source.config_yaml = source.config_yaml.replace(old, new);
        assert!(configuration::inspection_yaml(&source).is_err());
    }
}

#[test]
fn setup_package_changed_destination_does_not_inherit_a_credential() {
    let mut original = saved();
    original.providers[0].connection.credential_id = Some("saved-key".into());
    let mut same = saved();
    configuration::preserve_credentials(&mut same, &original);
    assert_eq!(
        same.providers[0].connection.credential_id.as_deref(),
        Some("saved-key")
    );
    let mut changed = saved();
    changed.providers[0].connection.base_url = "https://different.example.com/v1".into();
    configuration::preserve_credentials(&mut changed, &original);
    assert!(changed.providers[0].connection.credential_id.is_none());
}

#[test]
fn setup_package_review_releases_metadata_without_raw_certificate_or_configuration() {
    let package = saved();
    let dto =
        serde_json::to_value(configuration::dto(&package, &DesktopSettings::default()).unwrap())
            .unwrap();
    assert_eq!(dto["providers"][0]["credentialRequired"], true);
    assert!(dto.get("configYaml").is_none());
    assert!(dto.get("caPem").is_none());
    assert!(
        !serde_json::to_string(&dto)
            .unwrap()
            .contains("env:COMPANY_TOKEN")
    );
}

#[test]
fn setup_package_certificate_private_keys_and_bad_images_are_rejected() {
    assert!(archive::validate_ca("-----BEGIN PRIVATE KEY-----").is_err());
    let manifest = MANIFEST.replace(
        "displayName: Company AI",
        "displayName: Company AI\n    icon: assets/company.png",
    );
    assert!(
        archive::read(&zip(&[
            ("manifest.yaml", manifest.as_bytes()),
            ("config.yaml", CONFIG.as_bytes()),
            ("assets/company.png", b"not a png")
        ]))
        .is_err()
    );
}

#[test]
fn setup_package_metadata_survives_settings_serialization_without_activation() {
    let settings = DesktopSettings {
        setup_packages: vec![saved()],
        ..DesktopSettings::default()
    };
    let restored: DesktopSettings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    configuration::validate_saved(&restored.setup_packages).unwrap();
    assert!(!restored.managed_configured());
    assert!(restored.providers.is_empty());
    assert_eq!(restored.setup_packages[0].manifest.id, "company");
}

#[tokio::test]
#[ignore = "requires the prepared, verified Desktop sidecar"]
async fn native_setup_package_uses_runtime_yaml_validation_without_provider_requests() {
    let bundle = crate::bundle::VerifiedBundle::load().expect("prepared sidecar");
    for config in [
        CONFIG,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../examples/desktop-setup/config.yaml"
        )),
    ] {
        let mut source = archive::read(&bytes()).unwrap();
        source.config_yaml = config.into();
        if config != CONFIG {
            source.manifest = serde_saphyr::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../examples/desktop-setup/manifest.yaml"
            )))
            .unwrap();
        }
        let yaml = configuration::inspection_yaml(&source).unwrap();
        let response = colossus_sdk::inspect_sidecar_configuration(&bundle.sidecar, yaml)
            .await
            .unwrap();
        let canonical = response
            .canonical_config
            .expect("canonical provider/model configuration");
        let package = configuration::inspected(source, &canonical, &bytes()).unwrap();
        assert!(
            package
                .providers
                .iter()
                .all(|provider| provider.connection.credential_id.is_none())
        );
        if config == CONFIG {
            assert_eq!(package.providers.len(), 1);
        } else {
            assert_eq!(package.providers.len(), 5);
            assert_eq!(package.models.len(), 6);
            assert_eq!(package.roles["primary"], "engineering");
            assert!(package.models.iter().any(
                |model| model.profile == "engineering" && model.model == "company/engineering"
            ));
            assert_eq!(
                package
                    .providers
                    .iter()
                    .filter(|provider| provider.credential_slot.is_some())
                    .count(),
                4
            );
        }
    }
}

#[test]
fn setup_package_v6_settings_migrate_without_losing_configuration() {
    let mut value = serde_json::to_value(DesktopSettings::default()).unwrap();
    value["schemaVersion"] = json!(6);
    value["globalConfiguration"]["revision"] = json!(42);
    value.as_object_mut().unwrap().remove("setupPackages");
    let (settings, migrated) =
        crate::desktop_settings::decode_settings(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(migrated);
    assert_eq!(settings.global_configuration.revision, 42);
    assert!(settings.setup_packages.is_empty());
}

#[test]
fn setup_package_ca_is_optional_and_exports_only_public_certificates() {
    let mut params = rcgen::CertificateParams::new(vec!["company.example".into()]).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let key = rcgen::KeyPair::generate().unwrap();
    let pem = params.self_signed(&key).unwrap().pem();
    let manifest = format!("{MANIFEST}caBundle: certificates/company.pem\n");
    let bytes = zip(&[
        ("manifest.yaml", manifest.as_bytes()),
        ("config.yaml", CONFIG.as_bytes()),
        ("certificates/company.pem", pem.as_bytes()),
    ]);
    let source = archive::read(&bytes).unwrap();
    let ca = archive::validate_ca(source.ca_pem.as_deref().unwrap()).unwrap();
    assert_eq!(ca.len(), 1);
    let mut package = saved();
    package.ca_pem = Some(pem);
    package.manifest.ca_bundle = Some("certificates/company.pem".into());
    let view = configuration::dto(&package, &DesktopSettings::default()).unwrap();
    assert_eq!(view.certificate_fingerprints, ca.fingerprints_sha256());
    assert!(view.existing_certificate_fingerprints.is_empty());
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("BEGIN CERTIFICATE")
    );
    assert!(
        archive::read(&archive::write(&package).unwrap())
            .unwrap()
            .ca_pem
            .is_some()
    );
}

#[test]
fn setup_package_activation_requires_credentials_and_explicit_profile_replacement() {
    let mut package = saved();
    package.models.push(
        serde_json::from_value(json!({
            "profile":"engineering","providerProfile":"company","model":"company/engineering",
            "contextWindowTokens":32768,"maxOutputTokens":4096,
            "capabilities":{"toolCalls":true,"streaming":true,"imageInputs":false}
        }))
        .unwrap(),
    );
    let id = uuid::Uuid::now_v7().to_string();
    let mut settings = DesktopSettings {
        workspace: Some(crate::desktop_settings::WorkspaceSetting {
            id: id.clone(),
            path: std::path::PathBuf::from("unused-by-pure-validation"),
            identity: None,
            display_name: "Test".into(),
            display_path: "Test".into(),
        }),
        setup_packages: vec![package.clone()],
        ..DesktopSettings::default()
    };
    let mut request = super::commands::UseSetupModelInput {
        id: "company".into(),
        sha256: package.sha256,
        profile: "company".into(),
        model_profile: "engineering".into(),
        workspace_id: id,
        replace_conflicts: false,
    };
    assert!(super::commands::prepare_model(&settings, &request).is_err());
    settings.setup_packages[0].providers[0].credential_slot = None;
    let configured = super::commands::prepare_model(&settings, &request).unwrap();
    assert_eq!(configured.roles["primary"], "engineering");
    assert_eq!(configured.access_profile, settings.access_profile);
    assert_eq!(configured.execution_boundary, settings.execution_boundary);
    let mut conflict = settings.setup_packages[0].providers[0].connection.clone();
    conflict.base_url = "https://other.example.com/v1".into();
    settings.providers.push(conflict);
    assert!(super::commands::prepare_model(&settings, &request).is_err());
    request.replace_conflicts = true;
    assert!(super::commands::prepare_model(&settings, &request).is_ok());
    request.workspace_id = uuid::Uuid::now_v7().to_string();
    assert!(super::commands::prepare_model(&settings, &request).is_err());
}

#[test]
fn setup_package_workspace_export_replaces_host_handles_with_portable_slots() {
    let mut provider = saved().providers.remove(0).connection;
    provider.credential_id = Some("native-vault-handle".into());
    let settings = DesktopSettings {
        providers: vec![provider],
        ..DesktopSettings::default()
    };
    let exported = super::commands::export_current(&settings).unwrap();
    assert!(!exported.config_yaml.contains("native-vault-handle"));
    assert!(
        exported
            .config_yaml
            .contains("env:COLOSSUS_PROVIDER_1_TOKEN")
    );
    assert!(exported.providers[0].connection.credential_id.is_none());
}

#[test]
fn setup_package_rejects_referenced_parent_paths_and_inconsistent_saved_profiles() {
    let manifest = MANIFEST.replace(
        "displayName: Company AI",
        "displayName: Company AI\n    icon: assets/../company.png",
    );
    assert!(
        archive::read(&zip(&[
            ("manifest.yaml", manifest.as_bytes()),
            ("config.yaml", CONFIG.as_bytes()),
            ("assets/../company.png", b"invalid")
        ]))
        .is_err()
    );
    assert!(!valid_id("."));
    assert!(!valid_id(".."));
    let mut package = saved();
    package.providers[0].connection.profile = "missing".into();
    assert!(configuration::validate_saved(&[package]).is_err());
}

#[test]
fn setup_package_normalizes_packaged_png_icons_and_rejects_oversized_dimensions() {
    let manifest = MANIFEST.replace(
        "displayName: Company AI",
        "displayName: Company AI\n    icon: assets/company.png",
    );
    for size in [8, 513] {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            size,
            size,
            image::Rgba([20, 90, 180, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let result = archive::read(&zip(&[
            ("manifest.yaml", manifest.as_bytes()),
            ("config.yaml", CONFIG.as_bytes()),
            ("assets/company.png", png.get_ref()),
        ]));
        if size > 512 {
            assert!(result.is_err());
        } else {
            assert!(
                result.unwrap().icons["assets/company.png"].starts_with("data:image/png;base64,")
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires the prepared, verified Desktop sidecar"]
async fn native_setup_package_export_roundtrips_through_runtime_validation() {
    let bundle = crate::bundle::VerifiedBundle::load().expect("prepared sidecar");
    let mut provider = saved().providers.remove(0).connection;
    provider.credential_id = Some("local-vault-handle".into());
    let exported = super::commands::export_current(&DesktopSettings {
        providers: vec![provider],
        ..DesktopSettings::default()
    })
    .unwrap();
    let bytes = archive::write(&exported).unwrap();
    let source = archive::read(&bytes).unwrap();
    let yaml = configuration::inspection_yaml(&source).unwrap();
    let response = colossus_sdk::inspect_sidecar_configuration(&bundle.sidecar, yaml)
        .await
        .unwrap();
    let imported = configuration::inspected(
        source,
        &response.canonical_config.expect("exported YAML is valid"),
        &bytes,
    )
    .unwrap();
    assert!(imported.providers[0].credential_slot.is_some());
    assert!(imported.providers[0].connection.credential_id.is_none());
}

#[tokio::test]
#[ignore = "requires the prepared, verified Desktop sidecar"]
async fn native_setup_package_validation_does_not_hide_invalid_model_fields() {
    let bundle = crate::bundle::VerifiedBundle::load().expect("prepared sidecar");
    for suffix in [
        "models:\n  unexpected: true\n",
        "models:\n  profiles: {}\n  roles: {primary: missing}\n",
    ] {
        let mut source = archive::read(&bytes()).unwrap();
        source.config_yaml.push_str(suffix);
        let yaml = configuration::inspection_yaml(&source).unwrap();
        let response = colossus_sdk::inspect_sidecar_configuration(&bundle.sidecar, yaml)
            .await
            .unwrap();
        assert!(response.canonical_config.is_none());
    }
    let mut source = archive::read(&bytes()).unwrap();
    let presentation = source.manifest.providers.remove("company").unwrap();
    source
        .manifest
        .providers
        .insert("__setup_validation".into(), presentation);
    source.config_yaml = source
        .config_yaml
        .replace("    company:", "    __setup_validation:");
    let yaml = configuration::inspection_yaml(&source).unwrap();
    let response = colossus_sdk::inspect_sidecar_configuration(&bundle.sidecar, yaml)
        .await
        .unwrap();
    let package =
        configuration::inspected(source, &response.canonical_config.unwrap(), &bytes()).unwrap();
    assert_eq!(
        package.providers[0].connection.profile,
        "__setup_validation"
    );
}

#[test]
fn setup_package_archive_limits_are_enforced_before_parsing_members() {
    let oversized_archive = vec![0; archive::MAX_ARCHIVE_BYTES + 1];
    assert!(
        archive::read(&oversized_archive).err().unwrap().violations[0]
            .description
            .contains("2 MiB")
    );
    let oversized_member = vec![0; 256 * 1024 + 1];
    assert!(
        archive::read(&zip(&[("config.yaml", &oversized_member)]))
            .err()
            .unwrap()
            .violations[0]
            .description
            .contains("256 KiB")
    );
    let names = (0..41).map(|i| format!("entry{i}")).collect::<Vec<_>>();
    let entries = names
        .iter()
        .map(|name| (name.as_str(), b"x".as_slice()))
        .collect::<Vec<_>>();
    assert!(
        archive::read(&zip(&entries)).err().unwrap().violations[0]
            .description
            .contains("too many files")
    );
    let member = vec![0; 256 * 1024];
    let entries = names[..5]
        .iter()
        .map(|name| (name.as_str(), member.as_slice()))
        .collect::<Vec<_>>();
    assert!(
        archive::read(&zip(&entries)).err().unwrap().violations[0]
            .description
            .contains("expanded")
    );
}

#[test]
fn cancelled_setup_review_restores_saved_instructions_and_cannot_clear_a_newer_review() {
    use super::commands::{Review, SetupReviewState};
    let original = saved();
    let mut replacement = original.clone();
    replacement.sha256 = "b".repeat(64);
    replacement.manifest.description_markdown = "https://new.example.test/token".into();
    let settings = DesktopSettings {
        setup_packages: vec![original.clone()],
        ..DesktopSettings::default()
    };
    let reviews = SetupReviewState::default();
    *reviews.0.lock().unwrap() = Some(Review {
        package: replacement.clone(),
        previous_sha256: Some(original.sha256.clone()),
        certificate_fingerprints: Vec::new(),
    });
    reviews.cancel(&original.sha256).unwrap();
    assert_eq!(
        reviews.instructions(&settings, "company").unwrap(),
        replacement
    );
    reviews.cancel(&replacement.sha256).unwrap();
    assert!(reviews.0.lock().unwrap().is_none());
    assert_eq!(
        reviews.instructions(&settings, "company").unwrap(),
        original
    );
    assert!(
        reviews
            .instructions(&DesktopSettings::default(), "company")
            .is_err()
    );
}
