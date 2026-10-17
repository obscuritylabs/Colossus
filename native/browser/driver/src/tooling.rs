//! Reviewed NSS utility pin. Acquisition belongs to the explicit installer/builder.
use colossus_native_browser_pki::fingerprint;
use colossus_ports::BrowserDriverError;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};

#[derive(Deserialize)]
struct Artifact {
    sha256: String,
    source: String,
}
#[derive(Deserialize)]
struct Lock {
    artifacts: BTreeMap<String, Artifact>,
}

pub fn verify(path: &Path, expected: &str) -> Result<File, BrowserDriverError> {
    if !path.is_absolute()
        || path
            .canonicalize()
            .map_err(|_| BrowserDriverError::Denied)?
            != path
    {
        return Err(BrowserDriverError::Denied);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| BrowserDriverError::Unavailable)?;
    let metadata = file.metadata().map_err(|_| BrowserDriverError::Denied)?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 || metadata.mode() & 0o022 != 0 {
        return Err(BrowserDriverError::Denied);
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BrowserDriverError::Denied)?;
    if bytes.len() > 16 * 1024 * 1024
        || !bytes.starts_with(b"\x7fELF")
        || fingerprint(&bytes) != expected
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(file)
}

pub fn tools(
    certutil: &Path,
    certutil_sha: &str,
    pk12util: &Path,
    pk12util_sha: &str,
) -> Result<(File, File, Vec<File>), BrowserDriverError> {
    let lock: Lock = serde_json::from_str(include_str!("../../nss-tools.lock.json"))
        .map_err(|_| BrowserDriverError::Unavailable)?;
    let cert = lock
        .artifacts
        .get("certutil")
        .ok_or(BrowserDriverError::Unavailable)?;
    let pfx = lock
        .artifacts
        .get("pk12util")
        .ok_or(BrowserDriverError::Unavailable)?;
    if certutil_sha != cert.sha256 || pk12util_sha != pfx.sha256 {
        return Err(BrowserDriverError::Denied);
    }
    let mut libraries = Vec::new();
    for (name, artifact) in &lock.artifacts {
        if name.starts_with("lib") {
            libraries.push(verify(Path::new(&artifact.source), &artifact.sha256)?);
        }
    }
    Ok((
        verify(certutil, certutil_sha)?,
        verify(pk12util, pk12util_sha)?,
        libraries,
    ))
}
