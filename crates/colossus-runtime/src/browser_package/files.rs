//! Bounded read-only verification of compiler-bound adjacent browser payloads.
use super::manifest::{Acceptance, Release, TARGET, digest, signed, trust};
use colossus_bundles::BundleTrustStore;
use colossus_contracts::BundleManifest;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Component, Path},
};

const MAX_PAYLOAD: u64 = 4 * 1024 * 1024 * 1024;

/// SHA-256 of exact descriptor or payload bytes.
pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn regular(path: &Path, ceiling: u64) -> Result<File, &'static str> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|_| "bundled browser file is unavailable")?;
    let metadata = file
        .metadata()
        .map_err(|_| "bundled browser file is unavailable")?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || metadata.len() > ceiling
    {
        return Err("bundled browser file is unsafe or oversized");
    }
    Ok(file)
}
fn bytes(path: &Path, ceiling: u64) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    regular(path, ceiling)?
        .take(ceiling + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "bundled browser file could not be read")?;
    if bytes.len() as u64 > ceiling {
        return Err("bundled browser file exceeded bounds");
    }
    Ok(bytes)
}
fn file_hash(path: &Path, ceiling: u64) -> Result<(String, u64), &'static str> {
    let mut file = regular(path, ceiling)?;
    let before = file
        .metadata()
        .map_err(|_| "bundled browser file is unavailable")?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "bundled browser file could not be hashed")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > ceiling {
            return Err("bundled browser file exceeded bounds");
        }
        hasher.update(&buffer[..count]);
    }
    let after = file
        .metadata()
        .map_err(|_| "bundled browser file is unavailable")?;
    if total != before.len()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err("bundled browser file changed during verification");
    }
    Ok((hex::encode(hasher.finalize()), total))
}
fn bound(root: &Path, manifest: &BundleManifest, name: &str) -> Result<Vec<u8>, &'static str> {
    let entry = manifest
        .files
        .iter()
        .find(|entry| entry.path == name)
        .ok_or("browser publisher payload binding is missing")?;
    let value = bytes(
        &root.join(name),
        entry.size.ok_or("browser file size binding is absent")?,
    )?;
    if hash(&value) != entry.sha256 || Some(value.len() as u64) != entry.size {
        return Err("browser publisher payload bytes do not match");
    }
    Ok(value)
}
fn relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && !value.starts_with('/')
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Modes {
    desktop: bool,
    headless: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    schema_version: u32,
    protocol_version: u32,
    component: String,
    platform: String,
    target: String,
    executable: String,
    cef_version: String,
    chromium_version: String,
    archive_sha256: String,
    modes: Modes,
    files: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    kind: String,
    mode: u32,
    size: Option<u64>,
    sha256: Option<String>,
}

fn walk(
    root: &Path,
    directory: &Path,
    actual: &mut BTreeSet<String>,
    expected: &BTreeSet<String>,
) -> Result<(), &'static str> {
    for entry in
        std::fs::read_dir(directory).map_err(|_| "browser payload directory is unavailable")?
    {
        let entry = entry.map_err(|_| "browser payload directory is unavailable")?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|_| "browser payload entry is unavailable")?;
        if !kind.is_dir() && !kind.is_file() {
            return Err("browser payload contains a link or special file");
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "browser payload escaped its root")?
            .to_str()
            .ok_or("browser payload path is invalid")?
            .to_owned();
        if !expected.contains(&relative) {
            return Err("browser payload contains an unlisted entry");
        }
        if actual.len() >= 20_010 || !actual.insert(relative) {
            return Err("browser payload inventory exceeded bounds");
        }
        if kind.is_dir() {
            walk(root, &path, actual, expected)?;
        }
    }
    Ok(())
}

/// Independently verify sealed publisher bytes and every adjacent inventory entry.
pub fn verify(root: &Path, embedded: &[u8]) -> Result<Release, &'static str> {
    verify_with_trust(root, embedded, trust()?)
}

/// Verify a payload using an explicitly supplied public-key trust store.
/// Ordinary discovery always uses the independently compiled publisher key.
pub fn verify_with_trust(
    root: &Path,
    embedded: &[u8],
    trust: BundleTrustStore,
) -> Result<Release, &'static str> {
    if !root.is_absolute()
        || root
            .canonicalize()
            .map_err(|_| "bundled browser payload is absent")?
            != root
        || !root
            .symlink_metadata()
            .map_err(|_| "browser payload is unavailable")?
            .is_dir()
    {
        return Err("browser payload root is unsafe");
    }
    let manifest = signed(embedded, trust)?;
    if bytes(&root.join("manifest.json"), 1024 * 1024)? != embedded {
        return Err("adjacent browser publisher manifest does not match the binary");
    }
    let release: Release = serde_json::from_slice(&bound(root, &manifest, "browser-release.json")?)
        .map_err(|_| "browser release descriptor is invalid")?;
    release.validate()?;
    let acceptance: Acceptance =
        serde_json::from_slice(&bound(root, &manifest, "acceptance.json")?)
            .map_err(|_| "browser production acceptance receipt is invalid")?;
    acceptance.validate(&release)?;
    let inventory_bytes = bound(root, &manifest, "component/browser-component.json")?;
    if hash(&inventory_bytes) != release.component_manifest_sha256 {
        return Err("browser component inventory binding differs");
    }
    let inventory: Inventory = serde_json::from_slice(&inventory_bytes)
        .map_err(|_| "browser component inventory is invalid")?;
    if inventory.schema_version != 1
        || inventory.protocol_version != 1
        || inventory.component != "colossus-browser"
        || inventory.platform != "linux64"
        || inventory.target != TARGET
        || inventory.executable != "colossus-native-browser-host"
        || !inventory.modes.headless
        || inventory.modes.desktop
        || inventory.files.is_empty()
        || inventory.files.len() > 20_000
        || !digest(&inventory.archive_sha256)
        || inventory.chromium_version.len() > 64
        || release.capabilities.engine_version.as_deref() != Some(inventory.cef_version.as_str())
        || !inventory.files.iter().any(|entry| {
            entry.path == inventory.executable && entry.kind == "file" && entry.mode & 0o100 != 0
        })
    {
        return Err("browser component mode or version is unaccepted");
    }
    let mut expected = BTreeSet::from([
        "manifest.json".into(),
        "browser-release.json".into(),
        "acceptance.json".into(),
        "runtime-image.tar".into(),
        "component".into(),
        "component/browser-component.json".into(),
    ]);
    let mut total = 0_u64;
    for entry in inventory.files {
        if !relative(&entry.path)
            || entry.path == "browser-component.json"
            || entry.mode & !0o777 != 0
            || entry.mode & 0o022 != 0
            || !expected.insert(format!("component/{}", entry.path))
        {
            return Err("browser component entry is unsafe or duplicated");
        }
        let path = root.join("component").join(&entry.path);
        let metadata = path
            .symlink_metadata()
            .map_err(|_| "browser component entry is absent")?;
        if metadata.mode() & 0o777 != entry.mode {
            return Err("browser component entry mode changed");
        }
        match entry.kind.as_str() {
            "directory" if metadata.is_dir() && entry.size.is_none() && entry.sha256.is_none() => {}
            "file" if metadata.is_file() => {
                let (digest, size) = file_hash(&path, MAX_PAYLOAD)?;
                if Some(size) != entry.size || entry.sha256.as_deref() != Some(digest.as_str()) {
                    return Err("browser component bytes changed");
                }
                total = total
                    .checked_add(size)
                    .ok_or("browser component size overflow")?;
                if total > MAX_PAYLOAD {
                    return Err("browser component exceeded bounds");
                }
            }
            _ => return Err("browser component contains an invalid entry"),
        }
    }
    let mut actual = BTreeSet::new();
    walk(root, root, &mut actual, &expected)?;
    if expected != actual {
        return Err("browser payload includes absent or unlisted entries");
    }
    if file_hash(&root.join("runtime-image.tar"), MAX_PAYLOAD)?.0 != release.image_archive_sha256 {
        return Err("browser offline runtime image bytes changed");
    }
    Ok(release)
}
