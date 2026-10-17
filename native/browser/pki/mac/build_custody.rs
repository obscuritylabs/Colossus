use sha2::{Digest as _, Sha256};
use std::{
    env,
    error::Error,
    fs,
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};

pub fn seal() -> Result<(), Box<dyn Error>> {
    const TRUST: &str = "COLOSSUS_TLS_BROKER_MAC_PACKAGE_TRUST";
    const PARENT: &str = "COLOSSUS_TLS_BROKER_PARENT_CDHASH";
    println!("cargo:rerun-if-env-changed={TRUST}");
    println!("cargo:rerun-if-env-changed={PARENT}");
    println!("cargo:rerun-if-changed=build_custody.rs");
    if env::var_os("CARGO_FEATURE_MACOS_CUSTODY_DEVELOPMENT").is_none() {
        return Ok(());
    }
    let value = env::var(PARENT)?;
    if value.len() != 40
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || value.bytes().all(|b| b == b'0')
    {
        return Err("custody requires the independently compiled Core CDHash".into());
    }
    let mut parent = [0_u8; 20];
    for (index, byte) in parent.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[2 * index..2 * index + 2], 16)?;
    }
    let original =
        env::var_os(TRUST).ok_or("custody requires independent build-only package trust")?;
    let path = Path::new(&original);
    if !path.is_absolute() || path.canonicalize()? != path {
        return Err("custody package trust must have a canonical absolute path".into());
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let before = file.metadata()?;
    // SAFETY: geteuid takes no pointers and reads the build process's OS identity.
    let uid = unsafe { libc::geteuid() };
    if !before.is_file()
        || before.uid() != uid
        || before.nlink() != 1
        || before.mode() & 0o7777 != 0o600
        || !(1..=4096).contains(&before.len())
    {
        return Err("custody trust must be one owner-private bounded public file".into());
    }
    let mut bytes = Vec::new();
    file.by_ref().take(4097).read_to_end(&mut bytes)?;
    let after = fs::symlink_metadata(path)?;
    let identity = |m: &fs::Metadata| {
        (
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    };
    if after.file_type().is_symlink()
        || identity(&before) != identity(&after)
        || bytes.len() as u64 != before.len()
    {
        return Err("custody trust changed during compilation".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let object = value.as_object().ok_or("custody trust must be an object")?;
    let fields = [
        "schema_version",
        "purpose",
        "package_root",
        "publisher",
        "key_id",
        "public_key",
    ];
    if object.len() != fields.len()
        || fields.iter().any(|field| !object.contains_key(*field))
        || value["schema_version"] != 1
        || value["purpose"] != "colossus-macos-owned-process-development"
        || value["publisher"] != "colossus-macos-development"
        || !fields[1..].iter().all(|field| value[*field].is_string())
        || !Path::new(value["package_root"].as_str().unwrap_or("")).is_absolute()
        || value["key_id"].as_str().is_none_or(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        || value["public_key"]
            .as_str()
            .is_none_or(|s| s.is_empty() || s.len() > 64)
    {
        return Err("custody requires the exact six-field owned-package public trust".into());
    }
    let output = env::var("OUT_DIR")?;
    fs::write(Path::new(&output).join("custody-package-trust.json"), bytes)?;
    fs::write(
        Path::new(&output).join("custody-parent-seal.rs"),
        format!("const EXPECTED_PARENT_CDHASH: [u8; 20] = {parent:?};\n"),
    )?;
    let sources = [
        "custody_policy.c",
        "custody_policy.h",
        "fixed_policy.c",
        "front_loader.c",
        "policy_transition.c",
        "custody_probe.c",
        "custody_probe.h",
    ];
    let mut hashes = serde_json::Map::new();
    for name in sources {
        let path = Path::new("../../process-broker").join(name);
        let mut hash = String::new();
        for byte in Sha256::digest(fs::read(&path)?) {
            use std::fmt::Write as _;
            write!(&mut hash, "{byte:02x}")?;
        }
        hashes.insert(
            format!("native/browser/process-broker/{name}"),
            serde_json::Value::String(hash),
        );
        println!("cargo:rerun-if-changed={}", path.display());
    }
    fs::write(
        Path::new(&output).join("custody-policy-sources.json"),
        serde_json::to_vec(&hashes)?,
    )?;
    let controller = Path::new("../../process-broker/tests/custody_front_controller.c");
    let mut controller_hash = String::new();
    for byte in Sha256::digest(fs::read(controller)?) {
        use std::fmt::Write as _;
        write!(&mut controller_hash, "{byte:02x}")?;
    }
    fs::write(
        Path::new(&output).join("custody-controller-source.sha256"),
        controller_hash,
    )?;
    println!("cargo:rerun-if-changed={}", controller.display());
    println!("cargo:rerun-if-changed={}", path.display());
    Ok(())
}
