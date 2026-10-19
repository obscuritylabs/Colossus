use super::*;

fn manifest() -> Manifest {
    Manifest {
        schema_version: 1,
        protocol_version: 1,
        component: "colossus-browser".into(),
        platform: "windows64".into(),
        target: "x86_64-pc-windows-msvc".into(),
        executable: "colossus-native-browser-host.exe".into(),
        cef_version: "154.0.34+g14c5a08+chromium-154.0.8037.98".into(),
        chromium_version: "154.0.8037.98".into(),
        archive_sha256: "c81c34d048d87276f4a18f0b000206710a1709bb67dd658948b87100e1a538da".into(),
        modes: Modes {
            desktop: false,
            headless: false,
        },
        files: [
            "colossus-native-browser-host.exe",
            "colossus-native-browser-host.dll",
            "colossus-browser-helper.exe",
            "colossus-browser-helper.dll",
            "libcef.dll",
            "chrome_elf.dll",
        ]
        .into_iter()
        .map(|path| Entry {
            path: path.into(),
            kind: "file".into(),
            mode: 0o644,
            size: Some(1),
            sha256: Some("0".repeat(64)),
        })
        .collect(),
    }
}

#[test]
fn functional_preview_inventory_cannot_promote_production_modes() {
    let mut manifest = manifest();
    let mut capabilities = BrowserCapabilities::unavailable();
    capabilities.engine_version = Some(manifest.cef_version.clone());
    assert!(validate(&manifest, &capabilities, true).is_ok());
    assert!(validate(&manifest, &capabilities, false).is_err());
    manifest.modes.desktop = true;
    assert!(validate(&manifest, &capabilities, true).is_err());
    assert!(validate(&manifest, &capabilities, false).is_ok());
    manifest.modes.desktop = false;
    manifest.modes.headless = true;
    assert!(validate(&manifest, &capabilities, true).is_err());
}

#[test]
fn functional_preview_does_not_accept_other_protocols_or_engine_versions() {
    let mut manifest = manifest();
    let mut capabilities = BrowserCapabilities::unavailable();
    capabilities.engine_version = Some(manifest.cef_version.clone());
    manifest.protocol_version = 2;
    assert!(validate(&manifest, &capabilities, true).is_err());
    manifest.protocol_version = 1;
    manifest.cef_version = "unreviewed-native-version".into();
    capabilities.engine_version = Some(manifest.cef_version.clone());
    assert!(validate(&manifest, &capabilities, true).is_err());
}
