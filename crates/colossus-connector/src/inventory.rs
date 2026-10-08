//! Installation inventory identities; never derived from machine identifiers or paths.
use colossus_cloud_protocol::{DeploymentKind, RuntimeInventory, WorkspaceSharing};
use colossus_home::{ColossusHome, ConfinedRoot};
use fs4::fs_std::FileExt;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, Write},
    path::Path,
};

/// Return the native installation's stable opaque host identity and a workspace scope.
/// All CLI deployments and Desktop workspaces using this owner-private Colossus home
/// share a host group; the grouping never changes their independent grants.
pub fn native_inventory(
    workspace_id: String,
    workspace_label: String,
    deployment_kind: DeploymentKind,
) -> Result<RuntimeInventory, &'static str> {
    let home =
        ColossusHome::resolve_and_ensure().map_err(|_| "native host identity unavailable")?;
    let root = ConfinedRoot::bind(
        home.confined_root()
            .prepare_directory(Path::new("cloud-connector"))
            .map_err(|_| "native host identity unavailable")?,
    )
    .map_err(|_| "native host identity unavailable")?;
    let retained = root
        .open_file(Path::new("host-id"))
        .map_err(|_| "native host identity unavailable")?;
    let mut file = retained.file();
    FileExt::lock_exclusive(file).map_err(|_| "native host identity unavailable")?;
    let result = (|| {
        retained
            .revalidate(&root)
            .map_err(|_| "native host identity changed")?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(129)
            .read_to_end(&mut bytes)
            .map_err(|_| "native host identity unavailable")?;
        let host_id = if bytes.is_empty() {
            let id = format!("host-{}", uuid::Uuid::now_v7().simple());
            file.rewind()
                .map_err(|_| "native host identity unavailable")?;
            file.write_all(id.as_bytes())
                .map_err(|_| "native host identity unavailable")?;
            file.set_len(id.len() as u64)
                .map_err(|_| "native host identity unavailable")?;
            file.sync_all()
                .map_err(|_| "native host identity unavailable")?;
            id
        } else {
            String::from_utf8(bytes).map_err(|_| "native host identity invalid")?
        };
        let inventory = RuntimeInventory {
            policy: None,
            host_id,
            host_label: native_host_label(),
            platform: std::env::consts::OS.into(),
            deployment_kind,
            workspace_id: format!(
                "workspace-{}",
                hex::encode(Sha256::digest(workspace_id.as_bytes()))
            ),
            workspace_label,
            sharing: WorkspaceSharing::CloudOwned,
        };
        inventory
            .validate()
            .map_err(|_| "native inventory invalid")?;
        retained
            .revalidate(&root)
            .map_err(|_| "native host identity changed")?;
        Ok(inventory)
    })();
    FileExt::unlock(file).map_err(|_| "native host identity unavailable")?;
    result
}

/// Bounded native computer name for display; it never contributes to host identity.
pub fn native_host_label() -> String {
    display_host_label(sysinfo::System::host_name())
}

fn display_host_label(name: Option<String>) -> String {
    name.map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control))
        .unwrap_or_else(|| "Colossus host".into())
}

#[cfg(test)]
mod tests {
    use super::display_host_label;
    #[test]
    fn computer_names_are_bounded_display_metadata() {
        assert_eq!(
            display_host_label(Some("  Workstation  ".into())),
            "Workstation"
        );
        for name in [
            None,
            Some(String::new()),
            Some("bad\nname".into()),
            Some("a".repeat(257)),
        ] {
            assert_eq!(display_host_label(name), "Colossus host");
        }
    }
}
