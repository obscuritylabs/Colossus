use super::WindowsBrowserConfig;
use colossus_contracts::{BrowserCapabilities, BrowserMode};
use colossus_ports::BrowserDriverError;
use colossus_windows_native::BoundPath;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    io::Read as _,
    path::{Path, PathBuf},
};

pub(super) struct Installation {
    pub state: BoundPath,
    pub component: BoundPath,
    files: Vec<BoundPath>,
    pub digest: [u8; 32],
    pub capabilities: BrowserCapabilities,
    pub limits: WindowsBrowserConfig,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
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
struct Modes {
    desktop: bool,
    headless: bool,
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

pub(super) fn bind(
    config: WindowsBrowserConfig,
    diagnostic: bool,
) -> Result<Installation, BrowserDriverError> {
    if config.manifest_sha256 == [0; 32]
        || !config.capabilities.available
        || !config.capabilities.restrictive_egress
        || config.capabilities.private_ca_trust
        || config.capabilities.client_identities
        || config.capabilities.modes != [BrowserMode::Embedded]
        || !(256 * 1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&config.memory_bytes)
        || !(32..=512).contains(&config.max_processes)
        || config.egress.lifetime.as_millis() > u128::from(config.capabilities.limits.max_lease_ms)
    {
        return Err(BrowserDriverError::Unavailable);
    }
    let state =
        BoundPath::open_directory(&config.state_root).map_err(|_| BrowserDriverError::Denied)?;
    state
        .validate_private_owner_dacl()
        .map_err(|_| BrowserDriverError::Denied)?;
    state
        .validate_ancestor_namespace_authority()
        .map_err(|_| BrowserDriverError::Denied)?;
    let component = BoundPath::open_directory(&config.component_root)
        .map_err(|_| BrowserDriverError::Denied)?;
    component
        .validate_immutable_directory_dacl()
        .map_err(|_| BrowserDriverError::Denied)?;
    let inventory =
        BoundPath::open_immutable_file(&component.canonical_path().join("browser-component.json"))
            .map_err(|_| BrowserDriverError::Denied)?;
    let mut bytes = Vec::new();
    inventory
        .try_clone_file()
        .map_err(|_| BrowserDriverError::Denied)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BrowserDriverError::Denied)?;
    if bytes.len() > 8 * 1024 * 1024 || Sha256::digest(&bytes).as_slice() != config.manifest_sha256
    {
        return Err(BrowserDriverError::Denied);
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| BrowserDriverError::Denied)?;
    validate(&manifest, &config.capabilities, diagnostic)?;
    let mut expected = BTreeSet::from(["browser-component.json".to_owned()]);
    let mut files = vec![inventory];
    let mut total = 0_u64;
    for entry in manifest.files {
        if !relative(&entry.path)
            || !expected.insert(entry.path.to_ascii_lowercase())
            || entry.mode & !0o777 != 0
        {
            return Err(BrowserDriverError::Denied);
        }
        let path = component.canonical_path().join(&entry.path);
        match entry.kind.as_str() {
            "file" => {
                let file = BoundPath::open_immutable_file(&path)
                    .map_err(|_| BrowserDriverError::Denied)?;
                let mut source = file
                    .try_clone_file()
                    .map_err(|_| BrowserDriverError::Denied)?;
                let size = source
                    .metadata()
                    .map_err(|_| BrowserDriverError::Denied)?
                    .len();
                total = total
                    .checked_add(size)
                    .ok_or(BrowserDriverError::LimitExceeded)?;
                if size != entry.size.ok_or(BrowserDriverError::Denied)?
                    || total > 4 * 1024 * 1024 * 1024
                {
                    return Err(BrowserDriverError::Denied);
                }
                let mut digest = Sha256::new();
                let mut buffer = [0; 64 * 1024];
                loop {
                    let count = source
                        .read(&mut buffer)
                        .map_err(|_| BrowserDriverError::Denied)?;
                    if count == 0 {
                        break;
                    }
                    digest.update(&buffer[..count]);
                }
                if entry.sha256.as_deref() != Some(hex::encode(digest.finalize()).as_str()) {
                    return Err(BrowserDriverError::Denied);
                }
                files.push(file);
            }
            "directory" if entry.size.is_none() && entry.sha256.is_none() => {
                let directory =
                    BoundPath::open_directory(&path).map_err(|_| BrowserDriverError::Denied)?;
                directory
                    .validate_immutable_directory_dacl()
                    .map_err(|_| BrowserDriverError::Denied)?;
                files.push(directory);
            }
            _ => return Err(BrowserDriverError::Denied),
        }
    }
    if actual(component.canonical_path())? != expected {
        return Err(BrowserDriverError::Denied);
    }
    let mut digest = Sha256::new();
    digest.update(b"colossus-browser-windows-v1\0");
    digest.update(config.manifest_sha256);
    digest
        .update(serde_json::to_vec(&config.capabilities).map_err(|_| BrowserDriverError::Denied)?);
    digest.update(config.memory_bytes.to_be_bytes());
    digest.update(config.max_processes.to_be_bytes());
    digest.update(format!("{:?}", config.egress).as_bytes());
    Ok(Installation {
        state,
        component,
        files,
        digest: digest.finalize().into(),
        capabilities: config.capabilities.clone(),
        limits: config,
    })
}
impl Installation {
    pub(super) fn revalidate(&self) -> Result<(), BrowserDriverError> {
        for path in [&self.state, &self.component]
            .into_iter()
            .chain(&self.files)
        {
            path.revalidate().map_err(|_| BrowserDriverError::Denied)?;
        }
        self.component
            .validate_immutable_directory_dacl()
            .map_err(|_| BrowserDriverError::Denied)?;
        for directory in &self.files {
            if directory.canonical_path().is_dir() {
                directory
                    .validate_immutable_directory_dacl()
                    .map_err(|_| BrowserDriverError::Denied)?;
            }
        }
        let expected: BTreeSet<_> = self
            .files
            .iter()
            .map(|file| {
                file.canonical_path()
                    .strip_prefix(self.component.canonical_path())
                    .map(|value| {
                        value
                            .to_string_lossy()
                            .replace('\\', "/")
                            .to_ascii_lowercase()
                    })
                    .map_err(|_| BrowserDriverError::Denied)
            })
            .collect::<Result<_, _>>()?;
        if actual(self.component.canonical_path())? != expected {
            return Err(BrowserDriverError::Denied);
        }
        Ok(())
    }
}
fn validate(
    manifest: &Manifest,
    capabilities: &BrowserCapabilities,
    diagnostic: bool,
) -> Result<(), BrowserDriverError> {
    if manifest.schema_version != 1
        || manifest.protocol_version != 1
        || manifest.component != "colossus-browser"
        || manifest.platform != "windows64"
        || manifest.target != "x86_64-pc-windows-msvc"
        || manifest.executable != "colossus-native-browser-host.exe"
        || manifest.cef_version != "154.0.34+g14c5a08+chromium-154.0.8037.98"
        || manifest.chromium_version != "154.0.8037.98"
        || manifest.archive_sha256
            != "c81c34d048d87276f4a18f0b000206710a1709bb67dd658948b87100e1a538da"
        || capabilities.engine_version.as_deref() != Some(manifest.cef_version.as_str())
        || manifest.files.is_empty()
        || manifest.files.len() > 20_000
        || manifest.modes.headless
        || manifest.modes.desktop == diagnostic
        || [
            "colossus-native-browser-host.exe",
            "colossus-native-browser-host.dll",
            "colossus-browser-helper.exe",
            "colossus-browser-helper.dll",
            "libcef.dll",
            "chrome_elf.dll",
        ]
        .iter()
        .any(|required| {
            !manifest
                .files
                .iter()
                .any(|entry| entry.path == *required && entry.kind == "file")
        })
    {
        return Err(BrowserDriverError::Unavailable);
    }
    Ok(())
}
fn relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value.is_ascii()
        && !value.contains(['\\', ':'])
        && value.bytes().all(|byte| (32..127).contains(&byte))
        && value
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && !part.ends_with(['.', ' ']))
        && Path::new(value)
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}
fn actual(root: &Path) -> Result<BTreeSet<String>, BrowserDriverError> {
    let mut paths = BTreeSet::new();
    let mut directories = vec![PathBuf::from(root)];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).map_err(|_| BrowserDriverError::Denied)? {
            let entry = entry.map_err(|_| BrowserDriverError::Denied)?;
            let kind = entry.file_type().map_err(|_| BrowserDriverError::Denied)?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| BrowserDriverError::Denied)?
                .to_str()
                .ok_or(BrowserDriverError::Denied)?
                .replace('\\', "/")
                .to_ascii_lowercase();
            if kind.is_symlink()
                || (!kind.is_file() && !kind.is_dir())
                || paths.len() >= 20_001
                || !paths.insert(relative)
            {
                return Err(BrowserDriverError::Denied);
            }
            if kind.is_dir() {
                directories.push(path);
            }
        }
    }
    Ok(paths)
}
