use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_bundles::{bundle_signing_key_info, canonical_bundle_signing_bytes};
use colossus_contracts::{BundleFileEntry, BundleManifest, BundleSignature};
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest as _, Sha256};
use std::{
    error::Error,
    fs::OpenOptions,
    io::{Read as _, Write as _},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const PURPOSE: &str = "colossus-macos-owned-process-development";
const PUBLISHER: &str = "colossus-macos-development";

pub(super) fn run() -> Result<(), Box<dyn Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [mode, package_root, seed_path] = args.as_slice() else {
        return Err("expected trust|sign PACKAGE_ROOT PRIVATE_SEED_PATH".into());
    };
    let root = colossus_home::ConfinedRoot::bind(PathBuf::from(package_root))?;
    let seed_path = PathBuf::from(seed_path);
    if !seed_path.is_absolute() {
        return Err("development seed needs an absolute private path".into());
    }
    let seed_parent = colossus_home::ConfinedRoot::bind(
        seed_path.parent().ok_or("missing private seed directory")?,
    )?;
    let seed_path = seed_parent
        .path()
        .join(seed_path.file_name().ok_or("missing seed filename")?);
    if seed_path.starts_with(root.path()) {
        return Err("development seed must stay outside the entire browser allocation".into());
    }
    if mode == "trust" {
        let mut seed = Zeroizing::new([0; 32]);
        getrandom::fill(seed.as_mut())?;
        write_new(&seed_path, seed.as_ref())?;
        let key = bundle_signing_key_info(*seed);
        let trust = serde_json::json!({ "schema_version":1, "purpose":PURPOSE,
            "publisher":PUBLISHER, "package_root":root.path(),
            "key_id":key.key_id, "public_key":key.public_key });
        write_new(
            &root.path().join("compiler-trust.json"),
            &serde_json::to_vec(&trust)?,
        )?;
    } else if mode == "sign" {
        let seed = Zeroizing::new(read(&seed_path, 32)?);
        let value = Zeroizing::new(<[u8; 32]>::try_from(seed.as_slice())?);
        let key = bundle_signing_key_info(*value);
        let signer = SigningKey::from_bytes(&value);
        let trust: serde_json::Value =
            serde_json::from_slice(&read(&root.path().join("compiler-trust.json"), 4096)?)?;
        if trust["purpose"] != PURPOSE
            || trust["publisher"] != PUBLISHER
            || trust["package_root"] != serde_json::to_value(root.path())?
            || trust["key_id"] != key.key_id
            || trust["public_key"] != key.public_key
        {
            return Err(
                "development signer differs from the independently compiled public trust".into(),
            );
        }
        let descriptor = read(&root.path().join("owned-process-host.json"), 16 * 1024)?;
        let value: serde_json::Value = serde_json::from_slice(&descriptor)?;
        if value["schema_version"] != 1
            || value["purpose"] != PURPOSE
            || value["production_accepted"] != false
            || value["allocation_root"] != serde_json::to_value(root.path())?
        {
            return Err(
                "only exact non-production development allocation descriptors can be signed".into(),
            );
        }
        let mut manifest = BundleManifest {
            format_version: 1,
            name: PURPOSE.into(),
            version: "0.1.0".into(),
            publisher: PUBLISHER.into(),
            created_at: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)?,
            source_revision: None,
            files: vec![BundleFileEntry {
                path: "owned-process-host.json".into(),
                size: Some(descriptor.len() as u64),
                sha256: hex::encode(Sha256::digest(&descriptor)),
            }],
            signatures: Vec::new(),
        };
        manifest.signatures.push(BundleSignature {
            algorithm: "ed25519".into(),
            key_id: key.key_id,
            signature: BASE64.encode(
                signer
                    .sign(&canonical_bundle_signing_bytes(&manifest)?)
                    .to_bytes(),
            ),
        });
        root.revalidate()?;
        write_new(
            &root.path().join("macos-browser-development.bundle.json"),
            &serde_json::to_vec(&manifest)?,
        )?;
    } else {
        return Err("only trust and sign are supported".into());
    }
    root.revalidate()?;
    seed_parent.revalidate()?;
    println!("Mac owned-process development signing completed; production remains disabled");
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read(path: &Path, ceiling: u64) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)?;
    let before = file.metadata()?;
    if !before.is_file()
        || before.mode() & 0o077 != 0
        || before.nlink() != 1
        || before.len() > ceiling
        || before.uid() != rustix::process::geteuid().as_raw()
    {
        return Err("unsafe or oversized development signing file".into());
    }
    let mut bytes = Vec::new();
    (&mut file).take(ceiling + 1).read_to_end(&mut bytes)?;
    let after = std::fs::symlink_metadata(path)?;
    if after.file_type().is_symlink()
        || bytes.len() as u64 != before.len()
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
    {
        return Err("development signing file changed".into());
    }
    Ok(bytes)
}
