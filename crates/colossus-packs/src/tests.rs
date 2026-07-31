use super::{
    BUNDLE_MANIFEST, COLLECTION_MANIFEST, MAX_PACK_SKILL_REFERENCES, PACK_MANIFEST, PackError,
    PackExecutor, PackOperation, PackService, RELEASE_TARGETS, bundle_artifact_path,
    canonical_bundle_signing_bytes, canonical_collection_signing_bytes,
    canonical_pack_signing_bytes, current_release_target, digest_hex, extract_collection_archive,
    validate_pack_references, write_collection_archive,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, ActorType, BundleFileEntry, BundleManifest, CredentialReference, DecisionOutcome,
    PackFileEntry, PackManifest, PackMcpServerDeclaration, PackPathReference, PackSignature,
    PackStatus, PublisherTrust, SkillManifest,
};
use colossus_integrations::EventSourcedExtensionRepository;
use colossus_policy::{BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel, effect_request};
use colossus_ports::{EventJournal, ExtensionRepository};
use colossus_testkit::InMemoryEventJournal;
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write as _,
    path::Path,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

fn actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "pack-test".into(),
    }
}

fn repository() -> (
    Arc<InMemoryEventJournal>,
    Arc<EventSourcedExtensionRepository>,
) {
    let journal = Arc::new(InMemoryEventJournal::default());
    let repository = Arc::new(EventSourcedExtensionRepository::new(
        Arc::clone(&journal) as Arc<dyn EventJournal>
    ));
    (journal, repository)
}

fn write_pack(root: &Path) -> PackManifest {
    let docs = root.join("docs");
    fs::create_dir_all(&docs).expect("create docs");
    let body = b"verified pack documentation\n";
    fs::write(docs.join("README.md"), body).expect("write pack body");
    let manifest = PackManifest {
        format_version: 1,
        name: "demo-pack".into(),
        version: "0.1.0".into(),
        description: "A strict test pack.".into(),
        publisher: "example".into(),
        license: "Apache-2.0".into(),
        homepage: String::new(),
        capabilities: vec!["docs".into()],
        permissions: Vec::new(),
        files: vec![PackFileEntry {
            path: "docs/README.md".into(),
            sha256: hex::encode(Sha256::digest(body)),
            size: body.len() as u64,
            content_type: "text/markdown".into(),
        }],
        integrations: Vec::new(),
        skills: Vec::<PackPathReference>::new(),
        tools: Vec::new(),
        mcp_servers: Vec::new(),
        binaries: Vec::new(),
        docker: Vec::new(),
        docs: vec!["docs/README.md".into()],
        tests: Vec::new(),
        dependencies: Vec::new(),
        signatures: Vec::new(),
    };
    fs::write(
        root.join(PACK_MANIFEST),
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    manifest
}

#[test]
fn signed_pack_mcp_servers_reject_wildcard_tool_trust() {
    let directory = tempfile::tempdir().expect("pack directory");
    let mut manifest = write_pack(directory.path());
    manifest
        .capabilities
        .extend(["mcp_servers".into(), "binaries".into()]);
    manifest.permissions.push("process".into());
    manifest.binaries.push("docs/README.md".into());
    manifest.mcp_servers.push(PackMcpServerDeclaration {
        name: "remote-trust-is-forbidden".into(),
        command: "docs/README.md".into(),
        args: Vec::new(),
        env_refs: BTreeMap::new(),
        allowed_tools: vec!["*".into()],
        permissions: vec!["process".into()],
    });
    let error = validate_pack_references(
        directory.path(),
        &manifest,
        &BTreeSet::from(["docs/README.md".into()]),
    )
    .expect_err("pack wildcard must fail");
    assert!(error.to_string().contains("wildcard tool allowlist"));
}

fn write_pack_with_skill_references(root: &Path, count: usize) -> PackManifest {
    let mut manifest = write_pack(root);
    manifest.capabilities.push("skills".into());
    for index in 0..count {
        let skill_path = format!("skills/skill-{index:02}");
        let instructions = format!(
            "---\nname: skill-{index:02}\ndescription: Bounded pack skill {index}\n---\nUse this bounded pack skill safely.\n"
        );
        fs::create_dir_all(root.join(&skill_path)).expect("create pack skill");
        fs::write(
            root.join(&skill_path).join("SKILL.md"),
            instructions.as_bytes(),
        )
        .expect("write pack skill");
        manifest.skills.push(PackPathReference {
            path: skill_path.clone(),
        });
        manifest.files.push(PackFileEntry {
            path: format!("{skill_path}/SKILL.md"),
            sha256: hex::encode(Sha256::digest(instructions.as_bytes())),
            size: instructions.len() as u64,
            content_type: "text/markdown".into(),
        });
    }
    fs::write(
        root.join(PACK_MANIFEST),
        serde_json::to_vec_pretty(&manifest).expect("serialize skill pack manifest"),
    )
    .expect("write skill pack manifest");
    manifest
}

fn trust_key(
    repository: &dyn ExtensionRepository,
    publisher: &str,
    signing_key: &SigningKey,
) -> String {
    let public = signing_key.verifying_key().to_bytes();
    let key_id = digest_hex(&public);
    repository
        .add_publisher_trust(
            PublisherTrust {
                publisher: publisher.into(),
                key_id: key_id.clone(),
                public_key: BASE64.encode(public),
                added_at: "2026-07-11T00:00:00Z".into(),
            },
            actor(),
        )
        .expect("add trust");
    key_id
}

fn write_signed_pack(
    root: &Path,
    name: &str,
    version: &str,
    dependencies: Vec<String>,
    signing_key: &SigningKey,
    key_id: &str,
) {
    let mut manifest = write_pack(root);
    manifest.name = name.into();
    manifest.version = version.into();
    manifest.dependencies = dependencies;
    let unsigned = canonical_pack_signing_bytes(&manifest).expect("canonical pack");
    manifest.signatures.push(PackSignature {
        algorithm: "ed25519".into(),
        key_id: key_id.into(),
        signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
    });
    fs::write(
        root.join(PACK_MANIFEST),
        serde_json::to_vec_pretty(&manifest).expect("signed pack manifest"),
    )
    .expect("write signed pack");
}

fn write_skill(root: &Path, name: &str, version: &str) {
    fs::create_dir_all(root).expect("skill root");
    let manifest = SkillManifest {
        name: name.into(),
        version: version.into(),
        description: "Collection skill.".into(),
        triggers: vec![name.into()],
        required_tools: Vec::new(),
        permissions: Vec::new(),
        offline_compatible: true,
    };
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("skill manifest"),
    )
    .expect("write skill manifest");
    fs::write(root.join("SKILL.md"), "Use this data-only skill safely.\n")
        .expect("write skill instructions");
}

fn write_oci_layout(layout: &Path, pack: &Path, gzip: bool) {
    let mut tar_bytes = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut tar_bytes);
        archive
            .append_path_with_name(pack.join(PACK_MANIFEST), format!("demo/{PACK_MANIFEST}"))
            .expect("append manifest");
        archive
            .append_path_with_name(pack.join("docs/README.md"), "demo/docs/README.md")
            .expect("append body");
        archive.finish().expect("finish tar");
    }
    let (layer, media_type) = if gzip {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).expect("compress layer");
        (
            encoder.finish().expect("finish gzip"),
            "application/vnd.colossus.pack.v1.tar+gzip",
        )
    } else {
        (tar_bytes, "application/vnd.colossus.pack.v1.tar")
    };
    let blobs = layout.join("blobs/sha256");
    fs::create_dir_all(&blobs).expect("blobs");
    let layer_digest = hex::encode(Sha256::digest(&layer));
    fs::write(blobs.join(&layer_digest), &layer).expect("layer blob");
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "layers": [{
            "mediaType": media_type,
            "digest": format!("sha256:{layer_digest}"),
            "size": layer.len()
        }]
    }))
    .expect("OCI manifest");
    let manifest_digest = hex::encode(Sha256::digest(&manifest));
    fs::write(blobs.join(&manifest_digest), &manifest).expect("manifest blob");
    fs::write(
        layout.join("oci-layout"),
        br#"{"imageLayoutVersion":"1.0.0"}"#,
    )
    .expect("layout marker");
    fs::write(
        layout.join("index.json"),
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "manifests": [{
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "digest": format!("sha256:{manifest_digest}"),
                "size": manifest.len()
            }]
        }))
        .expect("index"),
    )
    .expect("index file");
}

#[test]
fn unsigned_pack_verifies_but_is_not_trusted() {
    let source = TempDir::new().expect("source");
    write_pack(source.path());
    let (_, repository) = repository();
    let service = PackService::new(repository, source.path().join("installed"));
    let evidence = service.verify(source.path()).expect("verify");
    assert_eq!(evidence.file_count, 1);
    assert!(!evidence.trusted);
    assert_eq!(evidence.manifest.name, "demo-pack");
}

#[test]
fn pack_skill_references_are_bounded() {
    let accepted = TempDir::new().expect("accepted source");
    write_pack_with_skill_references(accepted.path(), MAX_PACK_SKILL_REFERENCES);
    let (_, repository) = repository();
    let service = PackService::new(repository, accepted.path().join("installed"));
    let evidence = service
        .verify(accepted.path())
        .expect("verify bounded pack");
    assert_eq!(evidence.manifest.skills.len(), MAX_PACK_SKILL_REFERENCES);

    let rejected = TempDir::new().expect("rejected source");
    write_pack_with_skill_references(rejected.path(), MAX_PACK_SKILL_REFERENCES + 1);
    let error = service
        .verify(rejected.path())
        .expect_err("pack exceeding the skill-reference ceiling must fail");
    assert!(matches!(
        error,
        PackError::Invalid(message)
            if message == format!(
                "pack skills must contain at most {MAX_PACK_SKILL_REFERENCES} entries"
            )
    ));
}

#[test]
fn local_oci_tar_and_gzip_layouts_materialize_into_the_same_verified_pack() {
    let root = TempDir::new().expect("root");
    let pack = root.path().join("pack");
    fs::create_dir(&pack).expect("pack");
    write_pack(&pack);
    let (_, repository) = repository();
    let service = PackService::new(repository, root.path().join("installed"));
    for gzip in [false, true] {
        let layout = root
            .path()
            .join(if gzip { "gzip-layout" } else { "tar-layout" });
        fs::create_dir(&layout).expect("layout");
        write_oci_layout(&layout, &pack, gzip);
        let evidence = service.verify(&layout).expect("verify OCI layout");
        assert_eq!(evidence.manifest.name, "demo-pack");
        assert_eq!(evidence.file_count, 1);
    }
}

#[test]
fn oci_layer_link_entries_fail_before_pack_materialization() {
    let root = TempDir::new().expect("root");
    let layout = root.path().join("layout");
    let blobs = layout.join("blobs/sha256");
    fs::create_dir_all(&blobs).expect("blobs");
    let mut layer = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut layer);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_path("demo/link").expect("link path");
        header.set_link_name("../../outside").expect("link target");
        header.set_cksum();
        archive
            .append(&header, std::io::empty())
            .expect("append link");
        archive.finish().expect("finish tar");
    }
    let layer_digest = hex::encode(Sha256::digest(&layer));
    fs::write(blobs.join(&layer_digest), &layer).expect("layer");
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "layers": [{
            "mediaType": "application/vnd.colossus.pack.v1.tar",
            "digest": format!("sha256:{layer_digest}"),
            "size": layer.len()
        }]
    }))
    .expect("manifest");
    let manifest_digest = hex::encode(Sha256::digest(&manifest));
    fs::write(blobs.join(&manifest_digest), &manifest).expect("manifest blob");
    fs::write(
        layout.join("oci-layout"),
        br#"{"imageLayoutVersion":"1.0.0"}"#,
    )
    .expect("layout marker");
    fs::write(
        layout.join("index.json"),
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "manifests": [{
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "digest": format!("sha256:{manifest_digest}"),
                "size": manifest.len()
            }]
        }))
        .expect("index"),
    )
    .expect("index file");
    let (_, repository) = repository();
    let service = PackService::new(repository, root.path().join("installed"));
    assert!(matches!(
        service.verify(&layout),
        Err(PackError::Invalid(_))
    ));
}

#[test]
fn signed_pack_requires_the_exact_publisher_key_and_rejects_tampering() {
    let source = TempDir::new().expect("source");
    let mut manifest = write_pack(source.path());
    let (_, repository) = repository();
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let key_id = trust_key(repository.as_ref(), "example", &signing_key);
    let unsigned = canonical_pack_signing_bytes(&manifest).expect("canonical manifest");
    manifest.signatures.push(PackSignature {
        algorithm: "ed25519".into(),
        key_id: key_id.clone(),
        signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
    });
    fs::write(
        source.path().join(PACK_MANIFEST),
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write signed manifest");
    let service = PackService::new(repository, source.path().join("installed"));
    let evidence = service.verify(source.path()).expect("trusted verify");
    assert!(evidence.trusted);
    assert_eq!(evidence.trust_key_id.as_deref(), Some(key_id.as_str()));

    fs::write(
        source.path().join("docs/README.md"),
        b"tampered pack documentation\n",
    )
    .expect("tamper body");
    assert!(matches!(
        service.verify(source.path()),
        Err(PackError::Invalid(_))
    ));
}

#[test]
fn traversal_and_undeclared_payloads_fail_closed() {
    let parent = TempDir::new().expect("parent");
    let source = parent.path().join("pack");
    fs::create_dir(&source).expect("source");
    fs::write(parent.path().join("outside"), b"outside").expect("outside");
    let mut manifest = write_pack(&source);
    manifest.files[0] = PackFileEntry {
        path: "../outside".into(),
        sha256: hex::encode(Sha256::digest(b"outside")),
        size: 7,
        content_type: "application/octet-stream".into(),
    };
    manifest.docs = vec!["../outside".into()];
    fs::write(
        source.join(PACK_MANIFEST),
        serde_json::to_vec(&manifest).expect("serialize traversal"),
    )
    .expect("write traversal");
    let (_, repository) = repository();
    let service = PackService::new(repository, parent.path().join("installed"));
    assert!(matches!(
        service.verify(&source),
        Err(PackError::Invalid(_))
    ));

    write_pack(&source);
    fs::write(source.join("undeclared.bin"), b"hidden executable").expect("undeclared");
    assert!(matches!(
        service.verify(&source),
        Err(PackError::Invalid(_))
    ));
}

#[cfg(unix)]
#[test]
fn symlink_payload_is_rejected_even_when_it_points_inside_the_pack() {
    use std::os::unix::fs::symlink;

    let source = TempDir::new().expect("source");
    let mut manifest = write_pack(source.path());
    symlink("docs/README.md", source.path().join("alias.md")).expect("symlink");
    manifest.files.push(PackFileEntry {
        path: "alias.md".into(),
        sha256: manifest.files[0].sha256.clone(),
        size: manifest.files[0].size,
        content_type: "text/markdown".into(),
    });
    manifest.docs.push("alias.md".into());
    fs::write(
        source.path().join(PACK_MANIFEST),
        serde_json::to_vec(&manifest).expect("serialize symlink manifest"),
    )
    .expect("write manifest");
    let (_, repository) = repository();
    let service = PackService::new(repository, source.path().join("installed"));
    assert!(matches!(
        service.verify(source.path()),
        Err(PackError::Invalid(_))
    ));
}

#[test]
fn install_disable_enable_and_uninstall_are_event_sourced() {
    let root = TempDir::new().expect("root");
    let source = root.path().join("source");
    fs::create_dir(&source).expect("source");
    write_pack(&source);
    let (journal, repository) = repository();
    let service = PackService::new(repository.clone(), root.path().join("installed"));
    let installed = service
        .install(&source, true, actor())
        .expect("install unsigned with explicit override");
    assert_eq!(installed.status, PackStatus::Enabled);
    assert!(Path::new(&installed.installed_path).is_dir());
    assert_eq!(
        service
            .disable("demo-pack", actor())
            .expect("disable")
            .status,
        PackStatus::Disabled
    );
    assert_eq!(
        service.enable("demo-pack", actor()).expect("enable").status,
        PackStatus::Enabled
    );
    let removed = service.uninstall("demo-pack", actor()).expect("uninstall");
    assert_eq!(removed.status, PackStatus::Uninstalled);
    assert!(!Path::new(&removed.installed_path).exists());
    let event_types = journal
        .read_stream("pack:demo-pack")
        .expect("stream")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            "pack.installed.v1",
            "pack.disabled.v1",
            "pack.enabled.v1",
            "pack.uninstalled.v1"
        ]
    );
}

#[test]
fn signed_collection_build_is_reproducible_and_installs_dependency_order_without_clobbering() {
    let root = TempDir::new().expect("root");
    let source = root.path().join("source");
    let base = source.join("packs/base");
    let app = source.join("packs/app");
    let skill = source.join("skills/reviewer");
    fs::create_dir_all(&base).expect("base pack");
    fs::create_dir_all(&app).expect("app pack");
    write_skill(&skill, "reviewer", "1.0.0");
    let (journal, repository) = repository();
    let signing_key = SigningKey::from_bytes(&[31_u8; 32]);
    let key_id = trust_key(repository.as_ref(), "example", &signing_key);
    write_signed_pack(&base, "base", "1.0.0", Vec::new(), &signing_key, &key_id);
    write_signed_pack(
        &app,
        "app",
        "2.0.0",
        vec!["base@1.0.0".into()],
        &signing_key,
        &key_id,
    );
    let pack_root = root.path().join("installed-packs");
    let skill_root = root.path().join("installed-skills");
    let service =
        PackService::new(repository, pack_root.clone()).with_skill_install_root(skill_root.clone());
    let first = root.path().join("first");
    let second = root.path().join("second");
    let built = service
        .build_collection(
            &source,
            &first,
            "starter-kit",
            "1.0.0",
            "example",
            "2026-07-16T12:00:00Z",
            signing_key.to_bytes(),
        )
        .expect("build collection");
    service
        .build_collection(
            &source,
            &second,
            "starter-kit",
            "1.0.0",
            "example",
            "2026-07-16T12:00:00Z",
            signing_key.to_bytes(),
        )
        .expect("rebuild collection");
    assert_eq!(built.verification.manifest.artifacts.len(), 3);
    assert_eq!(
        built
            .verification
            .packs
            .iter()
            .map(|pack| pack.manifest.name.as_str())
            .collect::<Vec<_>>(),
        vec!["base", "app"]
    );
    assert_eq!(
        fs::read(first.join(COLLECTION_MANIFEST)).expect("first manifest"),
        fs::read(second.join(COLLECTION_MANIFEST)).expect("second manifest")
    );
    assert_eq!(
        canonical_collection_signing_bytes(&built.verification.manifest).expect("collection bytes"),
        canonical_collection_signing_bytes(
            &service
                .verify_collection(&second)
                .expect("second verification")
                .manifest
        )
        .expect("second collection bytes")
    );

    let installed = service
        .install_collection(&first, actor())
        .expect("install collection");
    assert_eq!(
        installed
            .packs
            .iter()
            .map(|pack| pack.manifest.name.as_str())
            .collect::<Vec<_>>(),
        vec!["base", "app"]
    );
    assert_eq!(installed.skills[0].name, "reviewer");
    assert!(pack_root.join("base/1.0.0").is_dir());
    assert!(pack_root.join("app/2.0.0").is_dir());
    assert!(skill_root.join("reviewer").is_dir());
    assert!(matches!(
        service.install_collection(&first, actor()),
        Err(PackError::Invalid(_))
    ));
    assert_eq!(
        journal.read_stream("pack:base").expect("base events").len(),
        1
    );
    assert_eq!(
        journal.read_stream("pack:app").expect("app events").len(),
        1
    );
    let reopened =
        EventSourcedExtensionRepository::new(Arc::clone(&journal) as Arc<dyn EventJournal>);
    assert_eq!(
        reopened
            .list_packs(10)
            .expect("reconstruct collection pack lifecycles")
            .iter()
            .map(|pack| pack.manifest.name.as_str())
            .collect::<Vec<_>>(),
        vec!["app", "base"]
    );

    fs::write(
        second.join("skills/reviewer/SKILL.md"),
        "tampered instructions\n",
    )
    .expect("tamper collection");
    assert!(matches!(
        service.verify_collection(&second),
        Err(PackError::Invalid(_))
    ));
}

#[test]
fn collection_rejects_incomplete_pack_dependency_closure() {
    let root = TempDir::new().expect("root");
    let source = root.path().join("source");
    let app = source.join("packs/app");
    fs::create_dir_all(&app).expect("app pack");
    let (_, repository) = repository();
    let signing_key = SigningKey::from_bytes(&[37_u8; 32]);
    let key_id = trust_key(repository.as_ref(), "example", &signing_key);
    write_signed_pack(
        &app,
        "app",
        "2.0.0",
        vec!["missing@1.0.0".into()],
        &signing_key,
        &key_id,
    );
    let service = PackService::new(repository, root.path().join("installed"));
    assert!(matches!(
        service.build_collection(
            &source,
            &root.path().join("collection"),
            "incomplete",
            "1.0.0",
            "example",
            "2026-07-16T12:00:00Z",
            signing_key.to_bytes(),
        ),
        Err(PackError::Invalid(_))
    ));
}

#[test]
fn collection_archive_is_deterministic_and_rejects_special_entries() {
    let root = TempDir::new().expect("root");
    let source = root.path().join("source");
    let skill = source.join("skills/reviewer");
    write_skill(&skill, "reviewer", "1.0.0");
    let (_, repository) = repository();
    let signing_key = SigningKey::from_bytes(&[41_u8; 32]);
    trust_key(repository.as_ref(), "example", &signing_key);
    let service = PackService::new(repository, root.path().join("installed"));
    let collection = root.path().join("collection");
    let built = service
        .build_collection(
            &source,
            &collection,
            "archive-test",
            "1.0.0",
            "example",
            "2026-07-16T12:00:00Z",
            signing_key.to_bytes(),
        )
        .expect("build collection");
    let first = root.path().join("first.tar");
    let second = root.path().join("second.tar");
    write_collection_archive(
        &collection,
        &built.verification,
        &mut fs::File::create(&first).expect("first archive"),
    )
    .expect("write first archive");
    write_collection_archive(
        &collection,
        &built.verification,
        &mut fs::File::create(&second).expect("second archive"),
    )
    .expect("write second archive");
    assert_eq!(
        fs::read(&first).expect("first"),
        fs::read(&second).expect("second")
    );
    let extracted = root.path().join("extracted");
    fs::create_dir(&extracted).expect("extracted root");
    extract_collection_archive(&first, &extracted).expect("extract archive");
    assert_eq!(
        service
            .verify_collection(&extracted)
            .expect("verify extracted")
            .manifest_sha256,
        built.verification.manifest_sha256
    );

    let hostile = root.path().join("hostile.tar");
    let output = fs::File::create(&hostile).expect("hostile archive");
    let mut archive = tar::Builder::new(output);
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o777);
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_link_name("../../outside").expect("link name");
    header.set_cksum();
    archive
        .append_data(&mut header, COLLECTION_MANIFEST, std::io::empty())
        .expect("hostile entry");
    archive.finish().expect("finish hostile archive");
    let hostile_destination = root.path().join("hostile-extracted");
    fs::create_dir(&hostile_destination).expect("hostile destination");
    assert!(matches!(
        extract_collection_archive(&hostile, &hostile_destination),
        Err(PackError::Invalid(_))
    ));
    assert!(!root.path().join("outside").exists());
}

#[tokio::test]
async fn authenticated_registry_push_and_pull_round_trip_through_effect_gateway() {
    const TOKEN_VARIABLE: &str = "PATH";
    assert!(!std::env::var(TOKEN_VARIABLE).expect("test PATH").is_empty());
    let root = TempDir::new().expect("root");
    let source = root.path().join("source");
    write_skill(&source.join("skills/reviewer"), "reviewer", "1.0.0");
    let (journal, repository) = repository();
    let signing_key = SigningKey::from_bytes(&[43_u8; 32]);
    trust_key(repository.as_ref(), "example", &signing_key);
    let service = Arc::new(PackService::new(
        repository,
        root.path().join("installed-packs"),
    ));
    let collection = root.path().join("collection");
    service
        .build_collection(
            &source,
            &collection,
            "registry-test",
            "1.0.0",
            "example",
            "2026-07-16T12:00:00Z",
            signing_key.to_bytes(),
        )
        .expect("build collection");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let origin = format!("http://{address}");
    let endpoint = format!("{origin}/collections/registry-test/1.0.0");
    let stored = Arc::new(Mutex::new(None::<Vec<u8>>));
    let server_stored = Arc::clone(&stored);
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 4096];
                let count = stream.read(&mut chunk).await.expect("read request");
                assert_ne!(count, 0, "request ended before headers");
                request.extend_from_slice(&chunk[..count]);
                if let Some(index) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8(request[..header_end].to_vec()).expect("headers");
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer ")
            );
            if headers.starts_with("PUT ") {
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .expect("content length");
                while request.len() - header_end < content_length {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).await.expect("read body");
                    assert_ne!(count, 0, "request ended before body");
                    request.extend_from_slice(&chunk[..count]);
                }
                *server_stored.lock().expect("stored") =
                    Some(request[header_end..header_end + content_length].to_vec());
                stream
                    .write_all(
                        b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .expect("write push response");
            } else {
                let body = server_stored
                    .lock()
                    .expect("stored")
                    .clone()
                    .expect("pushed body");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.colossus.collection.v1.tar\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write pull headers");
                stream.write_all(&body).await.expect("write pull body");
            }
        }
    });

    let policy = BuiltInPolicy::offline_default()
        .with_action("registry.push", DecisionOutcome::Allow)
        .with_action("registry.pull", DecisionOutcome::Allow)
        .with_filesystem_root(root.path().display().to_string(), "write")
        .with_environment(TOKEN_VARIABLE)
        .with_network_destination(&origin);
    let gateway = EffectGateway::new(
        Arc::clone(&journal) as Arc<dyn EventJournal>,
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["registry.push".into(), "registry.pull".into()]),
        [47_u8; 32],
    );
    let executor = PackExecutor::new(Arc::clone(&service));
    let credential_reference = format!("env:{TOKEN_VARIABLE}");
    let push = PackOperation::RegistryPush {
        path: collection.display().to_string(),
        url: endpoint.clone(),
        credential_reference: Some(credential_reference.clone()),
    };
    let mut request = effect_request(
        actor(),
        push.action(),
        push.resource(),
        serde_json::to_value(&push).expect("push operation"),
    );
    request.capabilities = vec![push.action().into()];
    request.credential_references = vec![CredentialReference {
        reference: credential_reference.clone(),
        value_hash: None,
    }];
    gateway
        .execute(request, &executor)
        .await
        .expect("push collection");

    let destination = root.path().join("pulled");
    let pull = PackOperation::RegistryPull {
        url: endpoint,
        destination: destination.display().to_string(),
        credential_reference: Some(credential_reference.clone()),
    };
    let mut request = effect_request(
        actor(),
        pull.action(),
        pull.resource(),
        serde_json::to_value(&pull).expect("pull operation"),
    );
    request.capabilities = vec![pull.action().into()];
    request.credential_references = vec![CredentialReference {
        reference: credential_reference,
        value_hash: None,
    }];
    gateway
        .execute(request, &executor)
        .await
        .expect("pull collection");
    server.await.expect("server");
    assert_eq!(
        service
            .verify_collection(&destination)
            .expect("verify pulled collection")
            .manifest_sha256,
        service
            .verify_collection(&collection)
            .expect("verify source collection")
            .manifest_sha256
    );
    let secret = std::env::var(TOKEN_VARIABLE).expect("test credential");
    let audit = serde_json::to_string(&journal.read_global(1, 200).expect("audit events"))
        .expect("audit JSON");
    assert!(!audit.contains(&secret));

    let mismatched_destination = root.path().join("mismatched");
    let mismatched = PackOperation::RegistryPull {
        url: format!("{origin}/never-contacted"),
        destination: mismatched_destination.display().to_string(),
        credential_reference: Some(format!("env:{TOKEN_VARIABLE}")),
    };
    let mut request = effect_request(
        actor(),
        mismatched.action(),
        mismatched.resource(),
        serde_json::to_value(&mismatched).expect("mismatched operation"),
    );
    request.capabilities = vec![mismatched.action().into()];
    request.credential_references = vec![CredentialReference {
        reference: "env:HOME".into(),
        value_hash: None,
    }];
    assert!(gateway.execute(request, &executor).await.is_err());
    assert!(!mismatched_destination.exists());
}

#[test]
fn offline_bundle_requires_a_valid_trusted_signature() {
    let source = TempDir::new().expect("bundle");
    fs::write(source.path().join("artifact.bin"), b"release bytes").expect("artifact");
    let (_, repository) = repository();
    let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
    let key_id = trust_key(repository.as_ref(), "colossus", &signing_key);
    let mut manifest = BundleManifest {
        format_version: 1,
        name: "colossus-offline".into(),
        version: "0.6.0".into(),
        publisher: "colossus".into(),
        created_at: "2026-07-11T00:00:00Z".into(),
        source_revision: Some("deadbeef".into()),
        files: vec![BundleFileEntry {
            path: "artifact.bin".into(),
            sha256: hex::encode(Sha256::digest(b"release bytes")),
            size: Some(13),
        }],
        signatures: Vec::new(),
    };
    let unsigned = canonical_bundle_signing_bytes(&manifest).expect("canonical bundle");
    manifest.signatures.push(PackSignature {
        algorithm: "ed25519".into(),
        key_id: key_id.clone(),
        signature: BASE64.encode(signing_key.sign(&unsigned).to_bytes()),
    });
    fs::write(
        source.path().join(BUNDLE_MANIFEST),
        serde_json::to_vec_pretty(&manifest).expect("serialize bundle"),
    )
    .expect("manifest");
    let service = PackService::new(repository, source.path().join("installed"));
    let evidence = service.verify_bundle(source.path()).expect("verify bundle");
    assert_eq!(evidence.trust_key_id, key_id);
    assert_eq!(evidence.total_bytes, 13);

    manifest.signatures[0].signature = BASE64.encode([0_u8; 64]);
    fs::write(
        source.path().join(BUNDLE_MANIFEST),
        serde_json::to_vec(&manifest).expect("serialize bad signature"),
    )
    .expect("bad manifest");
    assert!(matches!(
        service.verify_bundle(source.path()),
        Err(PackError::Invalid(_))
    ));
}

#[test]
fn signed_bundle_build_is_reproducible_and_installs_only_into_a_clean_prefix() {
    let root = TempDir::new().expect("root");
    let root = fs::canonicalize(root.path()).expect("canonical root");
    let source = root.join("staged");
    let target = current_release_target().expect("release target");
    let artifact = bundle_artifact_path(target);
    let artifact_path = source.join(&artifact);
    fs::create_dir_all(artifact_path.parent().expect("artifact parent"))
        .expect("artifact directory");
    fs::write(&artifact_path, b"standalone-native-binary").expect("artifact");
    fs::write(source.join("LICENSE"), b"Apache-2.0\n").expect("license");

    let (_, repository) = repository();
    let signing_key = SigningKey::from_bytes(&[33_u8; 32]);
    let key_id = trust_key(repository.as_ref(), "colossus", &signing_key);
    let service = PackService::new(repository, root.join("packs"));
    let first = root.join("bundle-one");
    let second = root.join("bundle-two");
    for destination in [&first, &second] {
        let materialization = service
            .build_bundle(
                &source,
                destination,
                "colossus-offline",
                "0.6.0",
                "colossus",
                "2026-07-11T00:00:00Z",
                Some("0123456789abcdef".into()),
                signing_key.to_bytes(),
            )
            .expect("build bundle");
        assert_eq!(materialization.signing_key_id, key_id);
        assert_eq!(materialization.targets, [target.to_owned()]);
        assert_eq!(materialization.verification.file_count, 2);
    }
    assert_eq!(
        fs::read(first.join(BUNDLE_MANIFEST)).expect("first manifest"),
        fs::read(second.join(BUNDLE_MANIFEST)).expect("second manifest")
    );

    let prefix = root.join("prefix");
    let installation = service
        .install_bundle(&first, &prefix)
        .expect("install bundle");
    assert_eq!(installation.target, target);
    assert_eq!(installation.artifact, artifact);
    assert_eq!(
        fs::read(&installation.installed_path).expect("installed bytes"),
        b"standalone-native-binary"
    );
    assert!(matches!(
        service.install_bundle(&first, &prefix),
        Err(PackError::Invalid(message))
            if message.contains("refuses to replace existing path")
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let actual_prefix = root.join("actual-prefix");
        fs::create_dir(&actual_prefix).expect("actual prefix");
        let linked_prefix = root.join("linked-prefix");
        symlink(&actual_prefix, &linked_prefix).expect("linked prefix");
        let error = service
            .install_bundle(&first, &linked_prefix)
            .expect_err("linked prefix must fail");
        assert!(
            error.to_string().contains("must be a real directory"),
            "{error}"
        );
    }

    let other_target = RELEASE_TARGETS
        .iter()
        .find(|candidate| **candidate != target)
        .expect("other release target");
    let other_source = root.join("other-staged");
    let other_artifact = other_source.join(bundle_artifact_path(other_target));
    fs::create_dir_all(other_artifact.parent().expect("other artifact parent"))
        .expect("other artifact directory");
    fs::write(&other_artifact, b"other-native-binary").expect("other artifact");
    let other_bundle = root.join("other-bundle");
    service
        .build_bundle(
            &other_source,
            &other_bundle,
            "colossus-offline-other",
            "0.6.0",
            "colossus",
            "2026-07-11T00:00:00Z",
            None,
            signing_key.to_bytes(),
        )
        .expect("build other-target bundle");
    let error = service
        .install_bundle(&other_bundle, &root.join("other-prefix"))
        .expect_err("wrong-target bundle must not install");
    assert!(
        error
            .to_string()
            .contains("does not contain a native executable"),
        "{error}"
    );

    fs::OpenOptions::new()
        .write(true)
        .open(first.join(&installation.artifact))
        .expect("open artifact for tampering")
        .write_all(b"tampered")
        .expect("tamper artifact");
    let error = service
        .verify_bundle(&first)
        .expect_err("tampered bundle must fail");
    assert!(error.to_string().contains("file hash mismatch"), "{error}");
}

#[cfg(unix)]
#[test]
fn signed_bundle_build_rejects_linked_staging_payloads() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().expect("root");
    let root = fs::canonicalize(root.path()).expect("canonical root");
    let source = root.join("staged");
    let artifact = source.join(bundle_artifact_path(
        current_release_target().expect("release target"),
    ));
    fs::create_dir_all(artifact.parent().expect("artifact parent")).expect("artifact directory");
    let outside = root.join("outside-binary");
    fs::write(&outside, b"outside").expect("outside binary");
    symlink(&outside, &artifact).expect("linked artifact");
    let (_, repository) = repository();
    let signing_key = SigningKey::from_bytes(&[34_u8; 32]);
    trust_key(repository.as_ref(), "colossus", &signing_key);
    let service = PackService::new(repository, root.join("packs"));
    let error = service
        .build_bundle(
            &source,
            &root.join("bundle"),
            "colossus-linked",
            "0.6.0",
            "colossus",
            "2026-07-11T00:00:00Z",
            None,
            signing_key.to_bytes(),
        )
        .expect_err("linked payload must fail");
    assert!(
        error.to_string().contains("symlink is forbidden"),
        "{error}"
    );
}

#[test]
fn offline_bundle_rejects_the_legacy_parent_traversal_shape() {
    let parent = TempDir::new().expect("parent");
    let source = parent.path().join("bundle");
    fs::create_dir(&source).expect("bundle");
    fs::write(parent.path().join("outside.bin"), b"outside").expect("outside");
    let manifest = BundleManifest {
        format_version: 1,
        name: "colossus-offline".into(),
        version: "0.6.0".into(),
        publisher: "colossus".into(),
        created_at: "2026-07-11T00:00:00Z".into(),
        source_revision: None,
        files: vec![BundleFileEntry {
            path: "../outside.bin".into(),
            sha256: hex::encode(Sha256::digest(b"outside")),
            size: Some(7),
        }],
        signatures: Vec::new(),
    };
    fs::write(
        source.join(BUNDLE_MANIFEST),
        serde_json::to_vec(&manifest).expect("serialize bundle"),
    )
    .expect("manifest");
    let (_, repository) = repository();
    let service = PackService::new(repository, parent.path().join("installed"));
    assert!(matches!(
        service.verify_bundle(&source),
        Err(PackError::Invalid(_))
    ));
}

#[cfg(unix)]
#[test]
fn offline_bundle_rejects_symlink_payloads() {
    use std::os::unix::fs::symlink;

    let parent = TempDir::new().expect("parent");
    let source = parent.path().join("bundle");
    fs::create_dir(&source).expect("bundle");
    fs::write(parent.path().join("outside.bin"), b"outside").expect("outside");
    symlink("../outside.bin", source.join("artifact.bin")).expect("symlink");
    let manifest = BundleManifest {
        format_version: 1,
        name: "colossus-offline".into(),
        version: "0.6.0".into(),
        publisher: "colossus".into(),
        created_at: "2026-07-11T00:00:00Z".into(),
        source_revision: None,
        files: vec![BundleFileEntry {
            path: "artifact.bin".into(),
            sha256: hex::encode(Sha256::digest(b"outside")),
            size: Some(7),
        }],
        signatures: Vec::new(),
    };
    fs::write(
        source.join(BUNDLE_MANIFEST),
        serde_json::to_vec(&manifest).expect("serialize bundle"),
    )
    .expect("manifest");
    let (_, repository) = repository();
    let service = PackService::new(repository, parent.path().join("installed"));
    assert!(matches!(
        service.verify_bundle(&source),
        Err(PackError::Invalid(_))
    ));
}
