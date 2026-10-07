//! Owner-private non-secret CLI lifecycle control; never authorizes cloud work.
use crate::{ConnectionConfig, ConnectorStatus};
use colossus_home::{ColossusHome, ConfinedRoot};
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    time::Duration,
};

type Error = Box<dyn std::error::Error>;
#[derive(Clone)]
pub(super) struct Control {
    root: ConfinedRoot,
    name: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Report {
    pub enrolled: bool,
    pub node_id: Option<String>,
    pub project_id: Option<String>,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub host_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    pub status: ConnectorStatus,
    pub run_instance: Option<String>,
    pub reported_at_ms: u64,
}
impl Report {
    pub fn saved(config: Option<&ConnectionConfig>) -> Self {
        Self {
            enrolled: config.is_some(),
            node_id: config.map(|config| config.node_id.clone()),
            project_id: config.map(|config| config.project_id.clone()),
            endpoint: config.map(|config| config.endpoint.clone()),
            host_id: config
                .and_then(|config| config.inventory.as_ref())
                .map(|inventory| inventory.host_id.clone()),
            workspace_id: config
                .and_then(|config| config.inventory.as_ref())
                .map(|inventory| inventory.workspace_id.clone()),
            status: if config.is_some_and(|config| config.revoked) {
                ConnectorStatus::Revoked
            } else {
                ConnectorStatus::Disconnected
            },
            run_instance: None,
            reported_at_ms: now(),
        }
    }
    fn live(&self) -> bool {
        self.run_instance.is_some()
            && now().saturating_sub(self.reported_at_ms) < 10_000
            && matches!(
                self.status,
                ConnectorStatus::Connected
                    | ConnectorStatus::Connecting
                    | ConnectorStatus::Reconnecting
            )
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stop {
    run_instance: String,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
impl Control {
    pub fn new(name: &str) -> Result<Self, Error> {
        colossus_ports::CredentialKey::new("cloud-connector", name)?;
        let home = ColossusHome::resolve_and_ensure()?;
        let root = ConfinedRoot::bind(
            home.confined_root()
                .prepare_directory(Path::new("cloud-connector/cli-control"))?,
        )?;
        Ok(Self {
            root,
            name: hex::encode(Sha256::digest(name.as_bytes())),
        })
    }
    fn path(&self, suffix: &str) -> PathBuf {
        PathBuf::from(format!("{}-{suffix}.json", self.name))
    }
    fn read<T: DeserializeOwned>(&self, suffix: &str) -> Result<Option<T>, Error> {
        let retained = match self.root.open_existing_file(&self.path(suffix)) {
            Ok(file) => file,
            Err(colossus_home::HomeError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let mut file = retained.file();
        FileExt::lock_shared(file)?;
        let result = (|| {
            retained.revalidate(&self.root)?;
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(16_385)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 16_384 {
                return Err("connector control record exceeds bound".into());
            }
            if bytes.is_empty() {
                return Ok(None);
            }
            let value = serde_json::from_slice(&bytes)?;
            retained.revalidate(&self.root)?;
            Ok(Some(value))
        })();
        FileExt::unlock(file)?;
        result
    }
    fn write<T: Serialize>(&self, suffix: &str, value: &T) -> Result<(), Error> {
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > 16_384 {
            return Err("connector control record exceeds bound".into());
        }
        let retained = self.root.open_file(&self.path(suffix))?;
        let mut file = retained.file();
        FileExt::lock_exclusive(file)?;
        let result = (|| {
            retained.revalidate(&self.root)?;
            file.rewind()?;
            file.write_all(&bytes)?;
            file.set_len(bytes.len() as u64)?;
            file.sync_data()?;
            retained.revalidate(&self.root)?;
            Ok(())
        })();
        FileExt::unlock(file)?;
        result
    }
    pub fn report(&self) -> Result<Option<Report>, Error> {
        self.read("status")
    }
    pub fn save(&self, report: &Report) -> Result<(), Error> {
        self.write("status", report)
    }
    pub fn stopped(&self, run: &str) -> Result<bool, Error> {
        Ok(self
            .read::<Stop>("stop")?
            .is_some_and(|request| request.run_instance == run))
    }
    pub async fn disconnect(&self) -> Result<(), Error> {
        let Some(report) = self.report()?.filter(Report::live) else {
            return Ok(());
        };
        let run = report
            .run_instance
            .ok_or("connector run identity unavailable")?;
        self.write(
            "stop",
            &Stop {
                run_instance: run.clone(),
            },
        )?;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if self
                .report()?
                .is_none_or(|report| report.run_instance.as_deref() != Some(&run) || !report.live())
            {
                return Ok(());
            }
        }
        Err("connector disconnect timed out; inspect its process before retrying".into())
    }
    pub fn output(report: &Report) -> serde_json::Value {
        let mut value = serde_json::to_value(report).unwrap_or(serde_json::Value::Null);
        if let Some(object) = value.as_object_mut() {
            object.remove("run_instance");
            object.insert(
                "status_fresh".into(),
                serde_json::Value::Bool(now().saturating_sub(report.reported_at_ms) < 10_000),
            );
            if report.run_instance.is_some()
                && !report.live()
                && !matches!(
                    report.status,
                    ConnectorStatus::Disconnected | ConnectorStatus::Revoked
                )
            {
                object.insert("status".into(), serde_json::Value::String("unknown".into()));
            }
        }
        value
    }
    pub fn heartbeat(&self, report: &mut Report, status: ConnectorStatus) -> Result<(), Error> {
        report.status = status;
        report.reported_at_ms = now();
        self.save(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_ignores_stale_stop_and_stale_status_is_unknown() {
        let temporary = tempfile::tempdir().unwrap();
        let control = Control {
            root: ConfinedRoot::bind(temporary.path().canonicalize().unwrap().join("control"))
                .unwrap(),
            name: "fixture".into(),
        };
        control
            .write(
                "stop",
                &Stop {
                    run_instance: "previous-run".into(),
                },
            )
            .unwrap();
        assert!(control.stopped("previous-run").unwrap());
        assert!(!control.stopped("new-run").unwrap());
        let mut report = Report::saved(None);
        report.run_instance = Some("new-run".into());
        report.status = ConnectorStatus::Connected;
        report.reported_at_ms = 0;
        control.save(&report).unwrap();
        let output = Control::output(&control.report().unwrap().unwrap());
        assert_eq!(output["status"], "unknown");
        assert_eq!(output["status_fresh"], false);
        assert!(output.get("run_instance").is_none());
    }
}
