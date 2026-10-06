//! Operator-owned audit export/verification and bounded operational maintenance.
use colossus_cloud::storage::{CloudMaintenancePolicy, CloudStore};
use colossus_cloud_postgres::{
    CloudDatabaseConfig, CloudPostgresStore, SignedCloudAuditCheckpoint,
    verify_checkpoint_signature,
};
use colossus_network::AdditionalRootCertificates;
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

struct Options {
    action: String,
    config: Option<PathBuf>,
    project: Option<String>,
    signing_variable: Option<String>,
    output: Option<PathBuf>,
    anchor: Option<PathBuf>,
    public_key: Option<String>,
}
fn options() -> Result<Options, &'static str> {
    let mut args = std::env::args().skip(1);
    let action = args.next().ok_or("action required")?;
    let mut options = Options {
        action,
        config: None,
        project: None,
        signing_variable: None,
        output: None,
        anchor: None,
        public_key: None,
    };
    while let Some(arg) = args.next() {
        let value = args.next().ok_or("option value required")?;
        match arg.as_str() {
            "--config" => options.config = Some(value.into()),
            "--project" => options.project = Some(value),
            "--audit-key-variable" => options.signing_variable = Some(value),
            "--output" => options.output = Some(value.into()),
            "--anchor" => options.anchor = Some(value.into()),
            "--public-key" => options.public_key = Some(value),
            _ => return Err("unknown option"),
        }
    }
    Ok(options)
}
fn read<T: serde::de::DeserializeOwned>(path: &Path, max: u64) -> Result<T, &'static str> {
    if std::fs::metadata(path)
        .map_err(|_| "file unavailable")?
        .len()
        > max
    {
        return Err("file bound exceeded");
    }
    serde_json::from_slice(&std::fs::read(path).map_err(|_| "file unavailable")?)
        .map_err(|_| "file format invalid")
}
fn write<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "output serialization failed")?;
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "new output file cannot be created")?;
    use std::io::Write;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "output write unavailable")
}
async fn run() -> Result<(), &'static str> {
    let options = options()?;
    if options.action == "verify-file" {
        let anchor: SignedCloudAuditCheckpoint = read(
            options.anchor.as_deref().ok_or("anchor required")?,
            32 * 1024 * 1024,
        )?;
        verify_checkpoint_signature(
            &anchor,
            options
                .public_key
                .as_deref()
                .ok_or("pinned public key required")?,
        )
        .map_err(|_| "checkpoint signature invalid")?;
        println!("checkpoint signature verified");
        return Ok(());
    }
    let config: CloudDatabaseConfig = read(
        options
            .config
            .as_deref()
            .ok_or("database reference config required")?,
        65536,
    )?;
    let store = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .map_err(|_| "database unavailable")?;
    match options.action.as_str() {
        "export" => {
            let checkpoint = store
                .export_audit_checkpoint(
                    options.project.as_deref().ok_or("project required")?,
                    options
                        .signing_variable
                        .as_deref()
                        .ok_or("distinct audit key reference required")?,
                )
                .await
                .map_err(|_| "checkpoint export unavailable")?;
            write(
                options
                    .output
                    .as_deref()
                    .ok_or("new output path required")?,
                &checkpoint,
            )?;
            println!(
                "{}",
                serde_json::json!({"checkpoint_sha256":checkpoint.checkpoint_sha256,"public_key":checkpoint.public_key,"heads":checkpoint.checkpoint.heads.len(),"verified_audit_records":checkpoint.checkpoint.verified_audit_records})
            );
        }
        "verify-db" => {
            let anchor: SignedCloudAuditCheckpoint = read(
                options.anchor.as_deref().ok_or("anchor required")?,
                32 * 1024 * 1024,
            )?;
            store
                .verify_audit_checkpoint(
                    &anchor,
                    options
                        .public_key
                        .as_deref()
                        .ok_or("pinned public key required")?,
                )
                .await
                .map_err(|_| "database differs from retained checkpoint")?;
            println!("database checkpoint verified");
        }
        "maintain" => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "clock unavailable")?
                .as_secs();
            let report = store
                .maintain(now, &CloudMaintenancePolicy::default())
                .await
                .map_err(|_| "bounded maintenance unavailable")?;
            println!(
                "{}",
                serde_json::to_string(&report).map_err(|_| "maintenance summary unavailable")?
            );
        }
        "cleanup-fixture" => {
            store
                .remove_fixture_schema(&config.schema)
                .await
                .map_err(|_| "selected schema is not an owned fixture")?;
            println!("selected generated fixture removed");
        }
        _ => {
            return Err(
                "action must be export, verify-file, verify-db, maintain or cleanup-fixture",
            );
        }
    }
    Ok(())
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("cloud audit helper failed: {error}");
        std::process::exit(1);
    }
}
