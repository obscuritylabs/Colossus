//! Protected instance custody and distributed edit admission; no synthetic project namespace.
use super::{HostedCatalog, HostedExecution, administrator};
use crate::{
    hosted::{management::SharedEdit, shared::SharedChat},
    server::State,
};
use colossus_cloud::{CloudError, CloudResult};
use colossus_contracts::VaultRecord;
use colossus_ports::{CredentialKey, CredentialVault};
use std::sync::Arc;

fn catalog_key() -> CloudResult<CredentialKey> {
    CredentialKey::new("cloud-hosted-catalog", "instance-v1").map_err(|_| CloudError::Storage)
}
pub(crate) fn catalog(host: &HostedExecution) -> CloudResult<HostedCatalog> {
    let vault = host.instance_vault()?;
    let Some(record) = vault
        .read(&catalog_key()?)
        .map_err(|_| CloudError::Storage)?
    else {
        return Ok(HostedCatalog::default());
    };
    serde_json::from_slice(record.expose()).map_err(|_| CloudError::Storage)
}
pub(super) fn save_catalog(
    host: &HostedExecution,
    edit: &InstanceEdit,
    actor: &str,
    catalog: &HostedCatalog,
) -> CloudResult<()> {
    let bytes =
        zeroize::Zeroizing::new(serde_json::to_vec(catalog).map_err(|_| CloudError::Storage)?);
    if bytes.len() > 524_288 {
        return Err(CloudError::ResourceExhausted);
    }
    let record = VaultRecord::new(bytes.to_vec()).map_err(|_| CloudError::ResourceExhausted)?;
    edit.vault(host, actor, "instance.catalog.replace")?
        .write(&catalog_key()?, &record)
        .map_err(|_| CloudError::Storage)
}

pub(super) struct InstanceEdit {
    shared: Option<Arc<SharedChat>>,
    owner: String,
    _projects: Vec<SharedEdit>,
}
impl InstanceEdit {
    pub(super) async fn begin(
        host: &HostedExecution,
        state: &State,
        actor: &str,
        drain: bool,
    ) -> CloudResult<Self> {
        let mut edit = Self {
            shared: host.shared.clone(),
            owner: format!("instance-edit-{}", uuid::Uuid::now_v7().simple()),
            _projects: Vec::new(),
        };
        if let Some(shared) = &edit.shared {
            shared.sql.claim_instance_gate(&edit.owner, 300)?;
        }
        if drain {
            for project in host.deployment.projects.keys() {
                let caller = administrator(project, actor)?;
                if let Some(project) = host.begin_shared_edit(state, &caller, true).await? {
                    edit._projects.push(project);
                } else {
                    if host.active_project_run_status(project).await? {
                        return Err(CloudError::Conflict);
                    }
                    host.stop_project(project).await?;
                }
            }
        }
        Ok(edit)
    }
    pub(super) fn vault(
        &self,
        host: &HostedExecution,
        actor: &str,
        operation: &str,
    ) -> CloudResult<Arc<dyn CredentialVault>> {
        if let Some(shared) = &self.shared {
            Ok(shared.instance_editing_vault(&self.owner, actor, operation))
        } else {
            host.instance_vault()
        }
    }
}
impl Drop for InstanceEdit {
    fn drop(&mut self) {
        self._projects.clear();
        if let Some(shared) = &self.shared {
            let _ = shared.sql.release_instance_gate(&self.owner);
        }
    }
}
