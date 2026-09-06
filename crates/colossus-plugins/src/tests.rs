use super::*;
use colossus_contracts::ActorType;
use flate2::{Compression, GzBuilder};
use tar::{EntryType, Header};
use tempfile::TempDir;

pub(crate) fn actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "user:test".into(),
    }
}

pub(crate) fn write_plugin(root: &Path) {
    fs::create_dir_all(root.join("skills/review/references")).expect("skill directories");
    fs::write(
        root.join("plugin.json"),
        r#"{
          "$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
          "name":"dev.example.review",
          "version":"1.2.3",
          "description":"Example plugin",
          "clientSpecific":"ignored"
        }"#,
    )
    .expect("plugin manifest");
    fs::write(
        root.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Review a repository safely.\nallowed-tools: shell filesystem\n---\nFollow the review checklist.\n",
    )
    .expect("skill");
    fs::write(
        root.join("skills/review/references/checklist.txt"),
        "Check tests and boundaries.\n",
    )
    .expect("resource");
    fs::write(
        root.join("mcp.json"),
        r#"{
          "$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
          "mcpServers": {
            "remote": {"type":"streamable-http","url":"https://mcp.example.test/api"},
            "legacy": {"type":"sse","url":"https://mcp.example.test/events"},
            "broken": {"type":"stdio"}
          }
        }"#,
    )
    .expect("mcp configuration");
}

#[test]
fn fixed_discovery_qualifies_skills_and_isolates_component_failures() {
    let temp = TempDir::new().expect("temp");
    write_plugin(temp.path());
    fs::create_dir_all(temp.path().join("unrelated/lowercase")).expect("unrelated directory");
    fs::write(
        temp.path().join("unrelated/lowercase/SKILL.md"),
        "---\nname: lowercase\ndescription: Must not load.\n---\nignored\n",
    )
    .expect("lowercase skill");

    let plugin = load_plugin(temp.path()).expect("plugin");
    assert_eq!(plugin.skills.len(), 1);
    assert_eq!(plugin.skills[0].id, "dev.example.review/review");
    assert_eq!(plugin.mcp_servers.len(), 1);
    assert_eq!(plugin.mcp_servers[0].id, "dev.example.review/remote");
    assert!(plugin.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unknown_manifest_field"
            && diagnostic.kind == PluginComponentKind::Plugin
    }));
    assert!(plugin.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unsupported_mcp_transport"
            && diagnostic.name.as_deref() == Some("legacy")
    }));
    assert!(plugin.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_mcp_server" && diagnostic.name.as_deref() == Some("broken")
    }));
}

#[test]
fn invalid_mcp_diagnostics_do_not_disclose_manifest_credential_values() {
    let temp = TempDir::new().expect("temp");
    write_plugin(temp.path());
    for document in [
        json!({"$schema": AGENT_PLUGIN_MCP_SCHEMA_V1, "mcpServers": {"broken": {"type": "stdio", "command": 42, "env": {"TOKEN": "fixture-secret-do-not-release"}}}}),
        json!({"$schema": AGENT_PLUGIN_MCP_SCHEMA_V1, "mcpServers": "fixture-secret-do-not-release"}),
    ] {
        fs::write(
            temp.path().join("mcp.json"),
            serde_json::to_vec(&document).expect("document"),
        )
        .expect("invalid manifest");
        let plugin = load_plugin(temp.path()).expect("isolated MCP failure");
        assert_eq!(plugin.skills.len(), 1);
        assert!(!plugin.diagnostics.is_empty());
        assert!(
            !serde_json::to_string(&plugin.diagnostics)
                .expect("inventory diagnostics")
                .contains("fixture-secret-do-not-release")
        );
    }
}

#[test]
fn progressive_disclosure_requires_qualified_selection() {
    let temp = TempDir::new().expect("temp");
    write_plugin(temp.path());
    let mut plugin = load_plugin(temp.path()).expect("plugin");
    plugin.installation.status = PluginStatus::Enabled;

    let discovery =
        compose_plugins(&[plugin.clone()], "base", &[], &[], true).expect("discovery composition");
    assert!(discovery.instructions.contains("dev.example.review/review"));
    assert!(
        !discovery
            .instructions
            .contains("Follow the review checklist")
    );

    let selected = compose_plugins(
        &[plugin.clone()],
        "base",
        &["dev.example.review/review".into()],
        &[],
        true,
    )
    .expect("selected composition");
    assert!(
        selected
            .instructions
            .contains("Follow the review checklist")
    );
    assert_eq!(
        selected.active_plugin_roots,
        vec![plugin.installation.root.clone()]
    );

    assert!(compose_plugins(&[plugin], "base", &["review".into()], &[], true).is_err());
}

#[test]
fn resources_are_contained_and_text_reads_are_bounded() {
    let temp = TempDir::new().expect("temp");
    write_plugin(temp.path());
    fs::write(
        temp.path().join("skills/review/binary.dat"),
        [0_u8, 0xff, 0_u8],
    )
    .expect("binary resource");
    let plugin = load_plugin(temp.path()).expect("plugin");
    let skill = &plugin.skills[0];
    let resources = list_resources(skill).expect("resources");
    assert!(
        resources
            .iter()
            .any(|entry| { entry.path == "references/checklist.txt" && entry.text })
    );
    assert!(
        resources
            .iter()
            .any(|entry| entry.path == "binary.dat" && !entry.text)
    );
    assert_eq!(
        read_resource(skill, "references/checklist.txt")
            .expect("text resource")
            .content,
        "Check tests and boundaries.\n"
    );
    assert!(read_resource(skill, "../SKILL.md").is_err());
    assert!(read_resource(skill, "binary.dat").is_err());
}

#[test]
fn strict_frontmatter_and_linked_payloads_are_rejected() {
    let temp = TempDir::new().expect("temp");
    write_plugin(temp.path());
    fs::write(
        temp.path().join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Invalid delimiter.\n---junk\ninstructions\n",
    )
    .expect("invalid skill");
    let plugin = load_plugin(temp.path()).expect("component failure only");
    assert!(plugin.skills.is_empty());
    assert!(
        plugin
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "invalid_skill")
    );

    write_plugin(temp.path());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            temp.path().join("plugin.json"),
            temp.path().join("linked-plugin.json"),
        )
        .expect("symlink");
        assert!(load_plugin(temp.path()).is_err());
    }
}

#[test]
fn oci_layout_and_archive_round_trip_deterministically() {
    let temp = TempDir::new().expect("temp");
    let plugin = temp.path().join("plugin");
    fs::create_dir(&plugin).expect("plugin root");
    write_plugin(&plugin);
    let first = build_plugin_artifact(&plugin).expect("first artifact");
    let second = build_plugin_artifact(&plugin).expect("second artifact");
    assert_eq!(first.manifest_digest, second.manifest_digest);
    assert_eq!(first.layer, second.layer);

    let layout = temp.path().join("layout");
    package_plugin_to_layout(&plugin, &layout, Some("v1")).expect("package");
    let verified = verify_plugin_layout(&layout, Some(&first.manifest_digest)).expect("verify");
    assert_eq!(verified.manifest_digest, first.manifest_digest);
    let archive = temp.path().join("layout.tar");
    export_layout_archive(&layout, &archive).expect("export");
    let imported = temp.path().join("imported");
    import_layout_archive(&archive, &imported).expect("import");
    assert_eq!(
        verify_plugin_layout(&imported, None)
            .expect("verify import")
            .manifest_digest,
        first.manifest_digest
    );
}

#[test]
fn machine_store_is_shared_by_home_and_preserves_run_snapshots() {
    let temp = TempDir::new().expect("temp");
    let plugin = temp.path().join("plugin");
    fs::create_dir(&plugin).expect("plugin root");
    write_plugin(&plugin);
    let home = temp.path().join("home");
    let first = PluginStore::new(&home).expect("first store");
    let second = PluginStore::new(&home).expect("second store");
    let installed = first.install_directory(&plugin, actor()).expect("install");
    assert!(
        first
            .snapshot(&[], &[])
            .expect("disabled snapshot")
            .is_empty()
    );
    second
        .enable(&installed.manifest.name, &installed.digest, true, actor())
        .expect("enable");
    let running = first.snapshot(&[], &[]).expect("active snapshot");
    assert_eq!(running.len(), 1);
    second
        .disable(&installed.manifest.name, actor())
        .expect("disable");
    assert!(second.snapshot(&[], &[]).expect("new snapshot").is_empty());
    assert_eq!(running.len(), 1, "existing run snapshot remains stable");

    let isolated = PluginStore::new(temp.path().join("other-home")).expect("isolated store");
    assert!(isolated.list(10).expect("isolated list").is_empty());
}

#[test]
fn garbage_collection_respects_live_snapshot_leases_and_removes_orphaned_blobs() {
    let temp = TempDir::new().expect("temp");
    let plugin = temp.path().join("plugin");
    fs::create_dir(&plugin).expect("plugin root");
    write_plugin(&plugin);
    let store = PluginStore::new(temp.path().join("home")).expect("store");
    let installed = store.install_directory(&plugin, actor()).expect("install");
    store
        .enable(&installed.manifest.name, &installed.digest, true, actor())
        .expect("enable");
    let (snapshot, lease) = store
        .snapshot_with_lease(&[], &[])
        .expect("leased snapshot");
    assert_eq!(snapshot.len(), 1);
    store
        .uninstall(&installed.manifest.name, &installed.digest, false, actor())
        .expect("uninstall");
    assert!(store.gc().expect("leased gc").is_empty());
    assert!(Path::new(&snapshot[0].installation.root).is_dir());

    drop(lease);
    assert_eq!(store.gc().expect("unleased gc"), vec![installed.digest]);
    assert!(!Path::new(&snapshot[0].installation.root).exists());
    assert!(
        fs::read_dir(store.root().join("blobs/sha256"))
            .expect("blob directory")
            .next()
            .is_none()
    );
}

#[test]
fn malicious_plugin_layers_are_rejected_and_cleaned_up() {
    let temp = TempDir::new().expect("temp");
    let plugin = temp.path().join("plugin");
    fs::create_dir(&plugin).expect("plugin root");
    write_plugin(&plugin);
    let artifact = build_plugin_artifact(&plugin).expect("artifact");
    let cases = [
        (
            "absolute",
            vec![("/escape", EntryType::Regular, 0, Vec::new())],
        ),
        (
            "traversal",
            vec![(
                "dev.example.review/../escape",
                EntryType::Regular,
                0,
                Vec::new(),
            )],
        ),
        (
            "duplicate",
            vec![
                ("dev.example.review/same", EntryType::Regular, 0, Vec::new()),
                ("dev.example.review/same", EntryType::Regular, 0, Vec::new()),
            ],
        ),
        (
            "symlink",
            vec![("dev.example.review/link", EntryType::Symlink, 0, Vec::new())],
        ),
        (
            "hardlink",
            vec![("dev.example.review/link", EntryType::Link, 0, Vec::new())],
        ),
        (
            "fifo",
            vec![("dev.example.review/pipe", EntryType::Fifo, 0, Vec::new())],
        ),
        (
            "character",
            vec![("dev.example.review/device", EntryType::Char, 0, Vec::new())],
        ),
        (
            "second-root",
            vec![("another/plugin.json", EntryType::Regular, 0, Vec::new())],
        ),
        (
            "file-directory",
            vec![
                ("dev.example.review/path", EntryType::Regular, 0, Vec::new()),
                (
                    "dev.example.review/path/file",
                    EntryType::Regular,
                    0,
                    Vec::new(),
                ),
            ],
        ),
        (
            "special",
            vec![("dev.example.review/device", EntryType::Block, 0, Vec::new())],
        ),
        (
            "oversized",
            vec![(
                "dev.example.review/huge",
                EntryType::Regular,
                MAX_FILE_BYTES + 1,
                Vec::new(),
            )],
        ),
    ];
    for (name, entries) in cases {
        let mut malicious = artifact.clone();
        malicious.layer = raw_plugin_layer(entries);
        // Authenticate the malicious bytes so this test reaches the archive parser,
        // rather than succeeding at the outer digest-mismatch rejection boundary.
        malicious.parsed_manifest.layers[0].digest = crate::common::sha256_digest(&malicious.layer);
        malicious.parsed_manifest.layers[0].size = malicious.layer.len() as u64;
        malicious.manifest =
            serde_json::to_vec(&malicious.parsed_manifest).expect("malicious manifest");
        malicious.manifest_digest = crate::common::sha256_digest(&malicious.manifest);
        let destination = temp.path().join(name);
        let error = extract_plugin_artifact(&malicious, &destination)
            .expect_err(name)
            .to_string();
        assert!(
            !error.contains("digest mismatch"),
            "must reach archive parsing for {name}: {error}"
        );
        assert!(
            !destination.exists(),
            "failed extraction must remove {name} staging content"
        );
    }
}

#[test]
fn oci_descriptors_and_single_layer_profile_are_enforced() {
    let temp = TempDir::new().expect("temp");
    let plugin = temp.path().join("plugin");
    fs::create_dir(&plugin).expect("plugin root");
    write_plugin(&plugin);
    let artifact = build_plugin_artifact(&plugin).expect("artifact");
    let mut multiple = artifact.parsed_manifest.clone();
    multiple.layers.push(multiple.layers[0].clone());
    assert!(crate::oci::validate_plugin_oci_manifest(&multiple).is_err());

    let layout = temp.path().join("layout");
    package_plugin_to_layout(&plugin, &layout, None).expect("layout");
    let config_hex = artifact.parsed_manifest.config.digest["sha256:".len()..].to_owned();
    let config_path = layout.join("blobs/sha256").join(config_hex);
    let mut bytes = fs::read(&config_path).expect("config blob");
    bytes.push(b'\n');
    fs::write(config_path, bytes).expect("tamper config blob");
    assert!(verify_plugin_layout(&layout, Some(&artifact.manifest_digest)).is_err());
}

#[test]
fn trust_modes_distinguish_required_optional_and_digest_only() {
    let temp = TempDir::new().expect("temp");
    let plugin = temp.path().join("plugin");
    fs::create_dir(&plugin).expect("plugin root");
    write_plugin(&plugin);
    let artifact = build_plugin_artifact(&plugin).expect("artifact");
    let required = PluginTrustProfile::default();
    assert!(verify_plugin_trust("required", &required, &artifact.manifest, &[]).is_err());

    let optional = PluginTrustProfile {
        mode: PluginTrustMode::Optional,
        ..PluginTrustProfile::default()
    };
    let evidence = verify_plugin_trust("optional", &optional, &artifact.manifest, &[])
        .expect("optional trust");
    assert!(!evidence.trusted);
    assert_eq!(evidence.method, "sigstore-unmatched");

    let disabled = PluginTrustProfile {
        mode: PluginTrustMode::Disabled,
        ..PluginTrustProfile::default()
    };
    let evidence = verify_plugin_trust("disabled", &disabled, &artifact.manifest, &[])
        .expect("disabled trust");
    assert!(!evidence.trusted);
    assert_eq!(evidence.method, "digest-only");
}

#[test]
fn docker_auth_requires_explicit_helper_mapping_and_parses_bounded_output() {
    let temp = TempDir::new().expect("temp");
    let config = temp.path().join("config.json");
    let helper = temp.path().join("docker-credential-test");
    fs::write(&helper, "helper").expect("helper placeholder");
    fs::write(
        &config,
        r#"{"credHelpers":{"registry.example.test":"test"}}"#,
    )
    .expect("docker config");
    let profile = PluginRegistryProfile {
        origin: "https://registry.example.test".into(),
        auth: RegistryAuthConfig::Docker {
            config_path: Some(config),
            helper_executables: BTreeMap::from([("test".into(), helper.clone())]),
        },
        trust_profile: "required".into(),
        ..PluginRegistryProfile::default()
    };
    assert_eq!(
        docker_credential_helper(&profile).expect("helper request"),
        Some((helper, "registry.example.test".into()))
    );
    assert_eq!(
        registry_credential_from_helper_output(br#"{"Username":"robot","Secret":"token"}"#)
            .expect("helper output"),
        RegistryCredential::Basic {
            username: "robot".into(),
            password: "token".into(),
        }
    );
    assert!(registry_credential_from_helper_output(br#"{"Secret":"token"}"#).is_err());
}

fn raw_plugin_layer(entries: Vec<(&str, EntryType, u64, Vec<u8>)>) -> Vec<u8> {
    let mut tar = Vec::new();
    append_raw_entry(&mut tar, "dev.example.review", EntryType::Directory, 0, &[]);
    for (path, kind, claimed_size, bytes) in entries {
        append_raw_entry(&mut tar, path, kind, claimed_size, &bytes);
    }
    tar.resize(tar.len() + 1024, 0);
    let mut gzip = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    gzip.write_all(&tar).expect("gzip tar");
    gzip.finish().expect("finish gzip")
}

fn append_raw_entry(
    tar: &mut Vec<u8>,
    path: &str,
    kind: EntryType,
    claimed_size: u64,
    bytes: &[u8],
) {
    assert!(path.len() < 100, "test path fits ustar header");
    let mut header = Header::new_gnu();
    header.as_mut_bytes()[..100].fill(0);
    header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
    header.set_entry_type(kind);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(claimed_size);
    header.set_cksum();
    tar.extend_from_slice(header.as_bytes());
    tar.extend_from_slice(bytes);
    let padding = (512 - bytes.len() % 512) % 512;
    tar.resize(tar.len() + padding, 0);
}
