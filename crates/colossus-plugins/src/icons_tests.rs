use super::*;
use tempfile::TempDir;

const ICON: &[u8] =
    include_bytes!("../../../bundled-plugins/colossus/com.obscuritylabs.colossus/icon.png");

fn fixture(icon: Value) -> TempDir {
    let temp = TempDir::new().expect("temporary plugin");
    crate::tests::write_plugin(temp.path());
    fs::create_dir(temp.path().join(NAMESPACE)).expect("icon directory");
    fs::write(temp.path().join(NAMESPACE).join("icon.png"), ICON).expect("icon");
    let manifest_path = temp.path().join("plugin.json");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read")).expect("json");
    manifest["extensions"] =
        json!({NAMESPACE: {"icon": icon}, "org.example.other": {"private": "not-in-inventory"}});
    fs::write(
        manifest_path,
        serde_json::to_vec(&manifest).expect("serialize"),
    )
    .expect("manifest");
    temp
}

#[test]
fn icon_survives_packaging_and_disabled_inventory_without_raw_extensions() {
    let temp = fixture(json!(format!("{NAMESPACE}/icon.png")));
    let home = TempDir::new().expect("home");
    let store = PluginStore::new(home.path().join("plugins-home")).expect("store");
    store
        .install_directory(temp.path(), crate::tests::actor())
        .expect("install");
    let inventory = store.inventory().expect("inventory");
    let entry = &inventory[0];
    assert!(!entry.available);
    assert!(entry.manifest.extensions.is_empty());
    let url = entry.icon_data_url.as_deref().expect("display icon");
    let bytes = STANDARD
        .decode(url.strip_prefix("data:image/png;base64,").expect("PNG URL"))
        .expect("base64");
    let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png).expect("PNG");
    assert_eq!((image.width(), image.height()), (128, 128));
}

#[test]
fn invalid_icons_are_isolated_from_skills_and_mcp() {
    for icon in [
        json!(42),
        json!("https://example.test/icon.png"),
        json!("/tmp/icon.png"),
        json!(format!("{NAMESPACE}/../private.png")),
        json!(format!("{NAMESPACE}//icon.png")),
        json!(format!("{NAMESPACE}/icon.svg")),
        json!(format!("{NAMESPACE}/missing.png")),
        json!("skills/review/icon.png"),
        json!(format!("{NAMESPACE}/C:\\private.png")),
    ] {
        let temp = fixture(icon);
        let record = load_plugin(temp.path()).expect("plugin remains loadable");
        assert!(record.icon_data_url.is_none());
        assert_eq!(record.skills.len(), 1);
        assert_eq!(record.mcp_servers.len(), 1);
        assert!(
            record
                .diagnostics
                .iter()
                .any(|d| d.code == "invalid_plugin_icon")
        );
    }
}

#[test]
fn icon_bytes_dimensions_and_decode_are_bounded() {
    let temp = fixture(json!(format!("{NAMESPACE}/icon.png")));
    let path = temp.path().join(NAMESPACE).join("icon.png");
    let mut oversized_dimensions = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(513, 1)
        .write_to(&mut oversized_dimensions, ImageFormat::Png)
        .expect("PNG");
    for bytes in [
        vec![0; MAX_ICON_BYTES as usize + 1],
        b"<svg onload='alert(1)'/>".to_vec(),
        ICON[..24].to_vec(),
        oversized_dimensions.into_inner(),
    ] {
        fs::write(&path, bytes).expect("invalid image");
        let record = load_plugin(temp.path()).expect("isolated image failure");
        assert!(record.icon_data_url.is_none());
        assert!(
            record
                .diagnostics
                .iter()
                .any(|d| d.code == "invalid_plugin_icon")
        );
    }
}

#[cfg(unix)]
#[test]
fn icon_reads_reject_links_even_within_the_plugin() {
    let temp = fixture(json!(format!("{NAMESPACE}/link.png")));
    std::os::unix::fs::symlink("icon.png", temp.path().join(NAMESPACE).join("link.png"))
        .expect("link");
    let (manifest, _) = load_manifest(temp.path()).expect("manifest");
    let mut diagnostics = Vec::new();
    assert!(
        load_icon(
            temp.path(),
            &manifest,
            &mut diagnostics,
            &mut IconBudget::default()
        )
        .is_none()
    );
    assert_eq!(diagnostics[0].code, "invalid_plugin_icon");
}

#[test]
fn absent_icon_keeps_legacy_plugins_loadable() {
    let temp = TempDir::new().expect("plugin");
    crate::tests::write_plugin(temp.path());
    let record = load_plugin(temp.path()).expect("legacy plugin");
    assert!(record.icon_data_url.is_none());
    assert!(
        !record
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_plugin_icon")
    );
}

fn png_bytes(image: &image::DynamicImage) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png).expect("PNG");
    output.into_inner()
}

#[test]
fn local_discovery_bounds_pixels_images_and_retained_bytes_before_loading() {
    let maximum_url = format!("data:image/png;base64,{}", STANDARD.encode(vec![0; 65_536]));
    assert_eq!(maximum_url.len(), 87_406);
    assert_eq!(
        CatalogIconBudget::default().bundled.bytes,
        maximum_url.len()
    );
    let mut random = 1_u32;
    let pixels = (0..127 * 127 * 4)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            random.to_le_bytes()[0]
        })
        .collect();
    let large = image::DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(127, 127, pixels).expect("pixels"),
    );
    for (image, expected) in [
        (image::DynamicImage::new_rgba8(512, 512), 31),
        (image::DynamicImage::new_rgba8(1, 1), 63),
        (large, 23),
    ] {
        let temp = fixture(json!(format!("{NAMESPACE}/icon.png")));
        fs::write(
            temp.path().join(NAMESPACE).join("icon.png"),
            png_bytes(&image),
        )
        .expect("icon");
        let mut budget = CatalogIconBudget::default();
        let mut retained = Vec::new();
        for _ in 0..100 {
            let record = load_plugin_with_icon_budget(
                temp.path(),
                budget.for_origin(PluginOrigin::Installed),
            )
            .expect("discovery");
            assert_eq!(record.skills.len(), 1);
            assert_eq!(record.mcp_servers.len(), 1);
            assert!(
                !record
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "invalid_plugin_icon")
            );
            retained.extend(record.icon_data_url);
        }
        assert_eq!(retained.len(), expected);
        let core =
            load_plugin_with_icon_budget(temp.path(), budget.for_origin(PluginOrigin::Bundled))
                .expect("reserved identity");
        retained.push(core.icon_data_url.expect("bundled icon"));
        assert!(retained.iter().map(String::len).sum::<usize>() <= MAX_INVENTORY_ICON_BYTES);
    }
    let temp = fixture(json!(format!("{NAMESPACE}/missing.png")));
    let record = load_plugin_with_icon_budget(temp.path(), &mut IconBudget::exhausted())
        .expect("no display I/O");
    assert!(
        !record
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_plugin_icon")
    );
    assert!(
        load_plugin(temp.path())
            .expect("full validation")
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_plugin_icon")
    );
}

#[test]
fn installed_catalogs_and_saved_snapshots_reserve_the_bundled_identity() {
    let temp = fixture(json!(format!("{NAMESPACE}/icon.png")));
    let home = TempDir::new().expect("home");
    let store = PluginStore::new(home.path().join("plugins-home")).expect("store");
    let manifest_path = temp.path().join("plugin.json");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read")).expect("json");
    let mut digests = BTreeMap::new();
    for index in 0..70 {
        manifest["name"] = json!(format!("a-plugin-{index:03}"));
        fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("manifest"),
        )
        .expect("write");
        let installed = store
            .install_directory(temp.path(), crate::tests::actor())
            .expect("install");
        store
            .enable(
                &installed.manifest.name,
                &installed.digest,
                true,
                crate::tests::actor(),
            )
            .expect("enable");
        digests.insert(installed.manifest.name, installed.digest);
    }
    manifest["name"] = json!("colossus");
    manifest
        .as_object_mut()
        .expect("object")
        .remove("clientSpecific");
    let manifest = serde_json::to_vec(&manifest).expect("bundled manifest");
    let artifact = build_plugin_artifact_from_files(&[
        PluginFile {
            path: "plugin.json",
            bytes: &manifest,
            executable: false,
        },
        PluginFile {
            path: "com.obscuritylabs.colossus/icon.png",
            bytes: ICON,
            executable: false,
        },
        PluginFile {
            path: "skills/review/SKILL.md",
            bytes: b"---\nname: review\ndescription: Review safely.\n---\nReview.",
            executable: false,
        },
    ])
    .expect("bundled artifact");
    let bundled = store
        .bootstrap_bundled(artifact, crate::tests::actor())
        .expect("bootstrap");
    digests.insert(bundled.manifest.name, bundled.digest);
    let inventory = store.inventory().expect("inventory");
    let snapshot = store
        .snapshot(&[], &[])
        .expect("snapshot")
        .iter()
        .map(AgentPluginRecord::inventory)
        .collect::<Vec<_>>();
    let (restored, _lease) = store
        .snapshot_digests_with_lease(&digests)
        .expect("saved snapshot");
    let restored = restored
        .iter()
        .map(AgentPluginRecord::inventory)
        .collect::<Vec<_>>();
    for entries in [inventory, snapshot, restored] {
        assert_eq!(entries.len(), 71);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.icon_data_url.is_some())
                .count(),
            64
        );
        assert!(entries.iter().all(|entry| entry.skills.len() == 1));
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.manifest.name.clone())
                .collect::<Vec<_>>(),
            digests.keys().cloned().collect::<Vec<_>>()
        );
        let core = entries
            .iter()
            .find(|entry| entry.origin == PluginOrigin::Bundled)
            .expect("core");
        assert!(core.icon_data_url.is_some());
        assert!(
            entries
                .iter()
                .filter_map(|entry| entry.icon_data_url.as_ref())
                .map(String::len)
                .sum::<usize>()
                <= MAX_INVENTORY_ICON_BYTES
        );
    }
}
