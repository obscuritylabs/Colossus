use crate::Status;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

pub(super) fn digest<const N: usize>(value: &str) -> Result<[u8; N], Status> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Status::Denied);
    }
    let bytes: [u8; N] = hex::decode(value)
        .map_err(|_| Status::Denied)?
        .try_into()
        .map_err(|_| Status::Denied)?;
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(Status::Denied);
    }
    Ok(bytes)
}
fn path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    (path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
        && !value
            .bytes()
            .any(|byte| byte < 32 || byte == b'"' || byte == b'\\')
        && !value.ends_with('/')
        && !value.contains("//"))
    .then(|| path.to_path_buf())
}
pub(super) fn policy_root(bytes: &[u8], allocation: &Path, app: &Path) -> Result<PathBuf, Status> {
    let text = std::str::from_utf8(bytes).map_err(|_| Status::Denied)?;
    let fields: Vec<_> = text.split_terminator('\n').collect();
    if !text.ends_with('\n')
        || fields.len() != 9
        || fields[0] != "COLOSSUS_MAC_NETWORK_ENVELOPE_V1"
        || fields[1]
            .parse::<u32>()
            .ok()
            .is_none_or(|n| matches!(n, 0 | u32::MAX) || n.to_string() != fields[1])
        || fields[2]
            .parse::<u16>()
            .ok()
            .is_none_or(|n| n == 0 || n.to_string() != fields[2])
        || Some(fields[3]) != allocation.to_str()
        || Some(fields[4]) != app.to_str()
        || Some(fields[6]) != allocation.join("profile").to_str()
        || fields[5].is_empty()
        || fields[5].len() > 160
        || !fields[5]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || path(fields[8]).is_none()
    {
        return Err(Status::Denied);
    }
    let root = path(fields[7]).ok_or(Status::Denied)?;
    if root.starts_with(allocation) || allocation.starts_with(&root) || root == Path::new("/") {
        return Err(Status::Denied);
    }
    Ok(root)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Denials {
    schema_version: u32,
    purpose: String,
    production_accepted: bool,
    component_manifest_sha256: String,
    policy_sha256: String,
    native_source_sha256: BTreeMap<String, String>,
    roles: Vec<Role>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Role {
    role: u8,
    cdhash: String,
    read_denied: bool,
    write_denied: bool,
    create_denied: bool,
    rename_denied: bool,
    unlink_denied: bool,
    link_denied: bool,
    extension_denied: bool,
    extension_use_denied: bool,
    security_mach_denied: bool,
    direct_child_reaped: bool,
    exit_code: u32,
}
pub(super) fn denials(
    bytes: &[u8],
    manifest: [u8; 32],
    policy: [u8; 32],
    roles: &[(u8, [u8; 20])],
    sources: &BTreeMap<String, String>,
) -> Result<(), Status> {
    let proof: Denials = serde_json::from_slice(bytes).map_err(|_| Status::Denied)?;
    if proof.schema_version != 1
        || proof.purpose != "colossus-macos-owned-process-custody-denials"
        || proof.production_accepted
        || digest::<32>(&proof.component_manifest_sha256)? != manifest
        || digest::<32>(&proof.policy_sha256)? != policy
        || proof.native_source_sha256 != *sources
        || proof.roles.len() != 5
        || roles.len() != 5
    {
        return Err(Status::Denied);
    }
    for (receipt, (role, hash)) in proof.roles.iter().zip(roles) {
        if receipt.role != *role
            || digest::<20>(&receipt.cdhash)? != *hash
            || ![
                receipt.read_denied,
                receipt.write_denied,
                receipt.create_denied,
                receipt.rename_denied,
                receipt.unlink_denied,
                receipt.link_denied,
                receipt.extension_denied,
                receipt.extension_use_denied,
                receipt.security_mach_denied,
                receipt.direct_child_reaped,
            ]
            .iter()
            .all(|n| *n)
            || receipt.exit_code != 0
        {
            return Err(Status::Denied);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_namespace_rejects_overlap_alias_spelling_and_extra_fields() {
        let source = "COLOSSUS_MAC_NETWORK_ENVELOPE_V1\n42\n1234\n/private/allocation\n/private/allocation/Browser.app\ncom.obscuritylabs.fixture\n/private/allocation/profile\n/private/custody\n/Users/fixture\n";
        let a = Path::new("/private/allocation");
        let b = Path::new("/private/allocation/Browser.app");
        assert_eq!(
            policy_root(source.as_bytes(), a, b).unwrap(),
            Path::new("/private/custody")
        );
        for root in [
            "/private/allocation/profile",
            "/private",
            "/",
            "/private/../custody",
            "/private//custody",
            "/private/custody/",
        ] {
            assert!(
                policy_root(source.replace("/private/custody", root).as_bytes(), a, b).is_err()
            );
        }
        assert!(policy_root(format!("{source}extra\n").as_bytes(), a, b).is_err());
        assert!(policy_root(source.trim_end().as_bytes(), a, b).is_err());
    }
    #[test]
    fn every_role_and_source_binding_is_mandatory() {
        let roles: Vec<_> = (0..5).map(|role| (role, [role + 1; 20])).collect();
        let sources = BTreeMap::from([("local.c".into(), "ab".repeat(32))]);
        let mut value = serde_json::json!({"schema_version":1,"purpose":"colossus-macos-owned-process-custody-denials","production_accepted":false,
            "component_manifest_sha256":"01".repeat(32),"policy_sha256":"02".repeat(32),"native_source_sha256":sources,
            "roles":roles.iter().map(|(role,hash)|serde_json::json!({"role":role,"cdhash":hex::encode(hash),"read_denied":true,"write_denied":true,"create_denied":true,"rename_denied":true,"unlink_denied":true,"link_denied":true,"extension_denied":true,"extension_use_denied":true,"security_mach_denied":true,"direct_child_reaped":true,"exit_code":0})).collect::<Vec<_>>()});
        assert!(
            denials(
                &serde_json::to_vec(&value).unwrap(),
                [1; 32],
                [2; 32],
                &roles,
                &sources
            )
            .is_ok()
        );
        value["roles"][2]["unlink_denied"] = false.into();
        assert!(
            denials(
                &serde_json::to_vec(&value).unwrap(),
                [1; 32],
                [2; 32],
                &roles,
                &sources
            )
            .is_err()
        );
        value["roles"][2]["unlink_denied"] = true.into();
        value["roles"][2]["cdhash"] = "ff".repeat(20).into();
        assert!(
            denials(
                &serde_json::to_vec(&value).unwrap(),
                [1; 32],
                [2; 32],
                &roles,
                &sources
            )
            .is_err()
        );
        assert!(
            denials(
                &serde_json::to_vec(&value).unwrap(),
                [1; 32],
                [2; 32],
                &roles,
                &BTreeMap::new()
            )
            .is_err()
        );
    }
}
