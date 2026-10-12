use colossus_mac_browser_process_broker::{DeploymentLaunch, DeploymentSetup};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read as _},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Component, Path},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: Vec<u8>,
    signature: Vec<u8>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    schema_version: u32,
    purpose: String,
    production_accepted: bool,
    pub controller_cdhash: [u8; 20],
    pub core_cdhash: [u8; 20],
    pub broker_cdhash: [u8; 20],
    pub setup: DeploymentSetup,
    files: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    sha256: [u8; 32],
}
impl Inventory {
    pub fn load() -> io::Result<Self> {
        let root = Path::new(crate::ROOT);
        if fs::canonicalize(root)? != root || !root.is_dir() {
            return Err(invalid("fixture root changed"));
        }
        let bytes = regular(root.join("inventory.json").as_path(), 256 * 1024)?;
        let envelope: Envelope = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("invalid signed fixture inventory"))?;
        let signature = Signature::from_slice(&envelope.signature)
            .map_err(|_| invalid("invalid fixture signature"))?;
        VerifyingKey::from_bytes(&crate::PUBLIC_KEY)
            .map_err(|_| invalid("invalid compiled fixture key"))?
            .verify_strict(&envelope.payload, &signature)
            .map_err(|_| invalid("fixture publisher rejected"))?;
        let value: Self = serde_json::from_slice(&envelope.payload)
            .map_err(|_| invalid("invalid fixture payload"))?;
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> io::Result<()> {
        if self.schema_version != 1
            || self.purpose != "colossus-known-supervisor-sentinel-v1"
            || self.production_accepted
            || [self.controller_cdhash, self.core_cdhash, self.broker_cdhash].contains(&[0; 20])
            || self.files.is_empty()
            || self.files.len() > 64
        {
            return Err(invalid("invalid fixture authority"));
        }
        self.setup.validate()?;
        let root = Path::new(crate::ROOT);
        let app = root.join("allocation/Colossus Custody Probe.app");
        let mut seen = std::collections::BTreeSet::new();
        for entry in &self.files {
            let relative = Path::new(&entry.path);
            if relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
                || !seen.insert(entry.path.as_str())
            {
                return Err(invalid("invalid fixed fixture path"));
            }
            let bytes = regular(&root.join(relative), 128 * 1024 * 1024)?;
            if Sha256::digest(bytes).as_slice() != entry.sha256 {
                return Err(invalid("fixture image or policy changed"));
            }
        }
        for name in [
            "controller",
            "core",
            "allocation/Colossus Custody Probe.app/Contents/MacOS/colossus-mac-browser-process-broker",
        ] {
            if !seen.contains(name) {
                return Err(invalid("missing supervisor fixture image"));
            }
        }
        for image in &self.setup.images {
            let fixed = if image.role == 0 {
                app.join("Contents/MacOS/colossus-native-browser-host")
            } else {
                let suffix = match image.role {
                    1 => " (Renderer)",
                    2 => " (GPU)",
                    3 | 4 => "",
                    _ => return Err(invalid("invalid fixture role")),
                };
                let name = format!("Colossus Browser Helper{suffix}");
                app.join(format!(
                    "Contents/Frameworks/{name}.app/Contents/MacOS/{name}"
                ))
            };
            if image.executable != fixed || fs::canonicalize(&fixed)? != fixed {
                return Err(invalid("role image escaped fixed package"));
            }
            let relative = fixed
                .strip_prefix(root)
                .map_err(|_| invalid("fixture role escaped allocation"))?
                .to_str()
                .ok_or_else(|| invalid("fixture role encoding"))?;
            if !seen.contains(relative) {
                return Err(invalid("missing sealed role front"));
            }
            let body = fixed.with_file_name(format!(
                "{} Body.dylib",
                fixed
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| invalid("role image name"))?
            ));
            let relative = body
                .strip_prefix(root)
                .map_err(|_| invalid("fixture body escaped allocation"))?
                .to_str()
                .ok_or_else(|| invalid("fixture body encoding"))?;
            if !seen.contains(relative)
                || Sha256::digest(regular(&body, 128 * 1024 * 1024)?).as_slice()
                    != image.body_sha256
            {
                return Err(invalid("missing sealed role body"));
            }
        }
        let policy = app.join("Contents/Resources/colossus-network-envelope.policy");
        if Sha256::digest(regular(&policy, 32768)?).as_slice() != self.setup.policy_sha256 {
            return Err(invalid("fixture policy changed"));
        }
        Ok(())
    }
    pub fn launch(&self) -> DeploymentLaunch {
        DeploymentLaunch { executable: Path::new(crate::ROOT).join("allocation/Colossus Custody Probe.app/Contents/MacOS/colossus-mac-browser-process-broker"), cdhash: self.broker_cdhash, setup:self.setup.clone() }
    }
}
pub fn regular(path: &Path, bound: u64) -> io::Result<Vec<u8>> {
    let before = fs::symlink_metadata(path)?;
    let file: File = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let state = file.metadata()?;
    // SAFETY: geteuid reads the current process identity with no pointer arguments.
    if !state.is_file()
        || state.uid() != unsafe { libc::geteuid() }
        || state.nlink() != 1
        || state.mode() & 0o022 != 0
        || state.len() > bound
        || before.dev() != state.dev()
        || before.ino() != state.ino()
    {
        return Err(invalid("unsafe fixture file"));
    }
    let mut bytes = Vec::new();
    file.take(bound + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > bound {
        return Err(invalid("oversized fixture file"));
    }
    Ok(bytes)
}
pub fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}
