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
    assert!(load_icon(temp.path(), &manifest, &mut diagnostics).is_none());
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
