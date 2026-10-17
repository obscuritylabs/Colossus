//! Compiler-bound browser discovery. No ambient executable, trust or image selector.
#![cfg(target_os = "linux")]
use colossus_runtime::browser_package::{files, manifest};
#[cfg(test)]
mod tests;

use serde_json::json;
use std::{
    error::Error,
    path::{Path, PathBuf},
};

const EMBEDDED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/browser-manifest.json"));

pub(super) type BrowserOwner =
    std::sync::Arc<colossus_runtime::browser_package::InstalledBrowserOwner>;

pub(super) async fn discover(state_parent: &Path) -> Result<Option<BrowserOwner>, Box<dyn Error>> {
    Ok(colossus_runtime::browser_package::discover(EMBEDDED, state_parent).await?)
}

/// Read-only packaging verification, handled before home/config/runtime acquisition.
/// The verification path never selects an ordinary runtime browser installation.
pub(super) fn packaging_command() -> Result<bool, Box<dyn Error>> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let Some(command) = arguments.first() else {
        return Ok(false);
    };
    if command != "__browser-bundle-info" && command != "__browser-bundle-verify" {
        return Ok(false);
    }
    if command == "__browser-bundle-info" {
        if arguments.len() != 1 {
            return Err("browser binding info accepts no arguments".into());
        }
        if EMBEDDED.is_empty() {
            println!(
                "{}",
                json!({"schema_version":1,"publisher_binding_present":false})
            );
        } else {
            let signed = manifest::signed(EMBEDDED, manifest::trust()?)?;
            println!(
                "{}",
                json!({"schema_version":1,"publisher_binding_present":true,
                "publisher_manifest_sha256":files::hash(EMBEDDED),"name":signed.name,
                "version":signed.version,"target":manifest::TARGET,"files":signed.files})
            );
        }
    } else {
        if arguments.len() != 2 {
            return Err("browser payload verification requires one directory".into());
        }
        let root = PathBuf::from(&arguments[1]).canonicalize()?;
        let release = files::verify(&root, EMBEDDED)?;
        println!(
            "{}",
            json!({"schema_version":1,"publisher_binding_present":true,
            "publisher_manifest_sha256":files::hash(EMBEDDED),"payload_verified":true,
            "publisher_acceptance_verified":true,"runtime_available":false,"image_id":release.image_id,
            "image_archive_sha256":release.image_archive_sha256,
            "component_manifest_sha256":release.component_manifest_sha256})
        );
    }
    Ok(true)
}
