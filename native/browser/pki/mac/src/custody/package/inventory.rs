use crate::Status;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Component, Path, PathBuf},
};
const NOFOLLOW: i32 = 0x0100; // Darwin O_NOFOLLOW from the pinned SDK.
const CLOEXEC: i32 = 0x01000000; // Darwin O_CLOEXEC.
type Identity = (u64, u64, u64, i64, i64, i64, i64, u32, u32, u64);
fn identity(m: &std::fs::Metadata) -> Identity {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
        m.mode(),
        m.uid(),
        m.nlink(),
    )
}
pub(super) struct HeldFile {
    path: PathBuf,
    file: File,
    identity: Identity,
}
impl HeldFile {
    pub fn read(
        path: &Path,
        limit: u64,
        private: bool,
        owner: u32,
    ) -> Result<(Self, Vec<u8>), Status> {
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW | CLOEXEC)
            .open(path)
            .map_err(|_| Status::Denied)?;
        let before = file.metadata().map_err(|_| Status::Denied)?;
        if !before.is_file()
            || before.uid() != owner
            || before.nlink() != 1
            || before.len() > limit
            || (private && before.mode() & 0o077 != 0)
        {
            return Err(Status::Denied);
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        file.by_ref()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Status::Denied)?;
        if bytes.len() as u64 != before.len() {
            return Err(Status::Denied);
        }
        let held = Self {
            path: path.to_path_buf(),
            file,
            identity: identity(&before),
        };
        held.revalidate()?;
        Ok((held, bytes))
    }
    pub fn revalidate(&self) -> Result<(), Status> {
        let held = self.file.metadata().map_err(|_| Status::Denied)?;
        let entry = std::fs::symlink_metadata(&self.path).map_err(|_| Status::Denied)?;
        if entry.file_type().is_symlink()
            || identity(&held) != self.identity
            || identity(&entry) != self.identity
        {
            return Err(Status::Denied);
        }
        Ok(())
    }
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
    source_kind: String,
    owned_process: serde_json::Value,
    modes: Modes,
    files: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Modes {
    desktop: bool,
    headless: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    kind: String,
    mode: Option<u32>,
    size: Option<u64>,
    sha256: Option<String>,
    target: Option<String>,
}
pub(super) fn verify(
    app: &Path,
    bytes: &[u8],
    sources: &BTreeMap<String, String>,
    owner: u32,
) -> Result<Vec<HeldFile>, Status> {
    let inventory: Inventory = serde_json::from_slice(bytes).map_err(|_| Status::Denied)?;
    if inventory.schema_version != 1
        || inventory.protocol_version != 1
        || inventory.component != "colossus-browser"
        || !matches!(
            (inventory.platform.as_str(), inventory.target.as_str()),
            ("macosarm64", "aarch64-apple-darwin") | ("macosx64", "x86_64-apple-darwin")
        )
        || inventory.executable != "Contents/MacOS/colossus-native-browser-host"
        || inventory.source_kind != "patched-chromium-development"
        || inventory.cef_version.is_empty()
        || inventory.chromium_version.is_empty()
        || inventory.modes.desktop
        || inventory.modes.headless
        || inventory.files.is_empty()
        || inventory.files.len() > 20_000
        || inventory.owned_process["development_only"] != true
        || [
            "production_accepted",
            "native_acceptance",
            "runtime_available",
            "complete_helper_cleanup_accepted",
        ]
        .iter()
        .any(|field| inventory.owned_process[*field] != false)
    {
        return Err(Status::Denied);
    }
    let embedded = inventory.owned_process["native_source_sha256"]
        .as_object()
        .ok_or(Status::Denied)?;
    if sources.iter().any(|(path, hash)| {
        embedded.get(path).and_then(serde_json::Value::as_str) != Some(hash.as_str())
    }) {
        return Err(Status::Denied);
    }
    let mut paths = BTreeSet::new();
    let mut folded = BTreeSet::new();
    let mut files = Vec::new();
    let mut total = 0_u64;
    for entry in &inventory.files {
        let relative = Path::new(&entry.path);
        if entry.path.is_empty()
            || entry.path.contains('\\')
            || relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || !paths.insert(entry.path.clone())
            || !folded.insert(entry.path.to_ascii_lowercase())
        {
            return Err(Status::Denied);
        }
        let path = app.join(relative);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| Status::Denied)?;
        if metadata.uid() != owner {
            return Err(Status::Denied);
        }
        if entry
            .mode
            .is_some_and(|mode| metadata.mode() & 0o7777 != mode)
        {
            return Err(Status::Denied);
        }
        match entry.kind.as_str() {
            "directory"
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && entry.size.is_none()
                    && entry.sha256.is_none()
                    && entry.target.is_none() => {}
            "file" => {
                let size = entry.size.ok_or(Status::Denied)?;
                total = total.checked_add(size).ok_or(Status::Denied)?;
                if total > 4 * 1024 * 1024 * 1024
                    || !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.nlink() != 1
                    || metadata.len() != size
                    || entry.target.is_some()
                {
                    return Err(Status::Denied);
                }
                let mut file = OpenOptions::new()
                    .read(true)
                    .custom_flags(NOFOLLOW | CLOEXEC)
                    .open(&path)
                    .map_err(|_| Status::Denied)?;
                let before = file.metadata().map_err(|_| Status::Denied)?;
                let mut sha = Sha256::new();
                let mut buffer = [0; 65536];
                let mut count = 0;
                loop {
                    let n = file.read(&mut buffer).map_err(|_| Status::Denied)?;
                    if n == 0 {
                        break;
                    }
                    count += n as u64;
                    if count > size {
                        return Err(Status::Denied);
                    }
                    sha.update(&buffer[..n]);
                }
                if count != size
                    || sha.finalize().as_slice()
                        != super::proof::digest::<32>(
                            entry.sha256.as_deref().ok_or(Status::Denied)?,
                        )?
                    || identity(&before) != identity(&file.metadata().map_err(|_| Status::Denied)?)
                {
                    return Err(Status::Denied);
                }
                // Keep exact authority-bearing roles/resources without retaining
                // thousands of framework descriptors throughout each mutation.
                if entry.path.contains("/MacOS/")
                    || entry.path == "Contents/Resources/owned-process-development.json"
                    || entry.path == "Contents/Resources/colossus-network-envelope.policy"
                {
                    let held = HeldFile {
                        path,
                        file,
                        identity: identity(&before),
                    };
                    held.revalidate()?;
                    files.push(held);
                }
            }
            "symlink" => {
                let target = entry.target.as_deref().ok_or(Status::Denied)?;
                if !metadata.file_type().is_symlink()
                    || entry.size.is_some()
                    || entry.mode.is_some()
                    || std::fs::read_link(&path)
                        .map_err(|_| Status::Denied)?
                        .to_str()
                        != Some(target)
                    || Sha256::digest(target.as_bytes()).as_slice()
                        != super::proof::digest::<32>(
                            entry.sha256.as_deref().ok_or(Status::Denied)?,
                        )?
                {
                    return Err(Status::Denied);
                }
            }
            _ => return Err(Status::Denied),
        }
    }
    let mut required = vec![
        "Contents/MacOS/colossus-native-browser-host".to_owned(),
        "Contents/MacOS/colossus-native-browser-host Body.dylib".to_owned(),
        "Contents/MacOS/colossus-mac-browser-process-broker".to_owned(),
        "Contents/MacOS/colossus-native-browser-tls-broker".to_owned(),
        "Contents/Resources/colossus-network-envelope.policy".to_owned(),
        "Contents/Resources/owned-process-development.json".to_owned(),
    ];
    for suffix in ["", " (Alerts)", " (GPU)", " (Plugin)", " (Renderer)"] {
        let name = format!("Colossus Browser Helper{suffix}");
        let front = format!("Contents/Frameworks/{name}.app/Contents/MacOS/{name}");
        required.push(front.clone());
        required.push(format!("{front} Body.dylib"));
    }
    if required.iter().any(|path| !paths.contains(path)) {
        return Err(Status::Denied);
    }
    Ok(files)
}
