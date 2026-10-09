//! Project names and structural hierarchy, independent of membership inheritance.
use super::accounts::{administrator, mutation, timestamp};
use super::*;
use crate::{CloudProject, CloudUser, project_key, validate_project};
fn decode_project(record: crate::storage::EntityRecord) -> CloudResult<CloudProject> {
    let mut project = CloudProject::try_from(record.value)?;
    if project.id != record.key.id {
        return Err(CloudError::Storage);
    }
    project.revision = record.revision;
    Ok(project)
}

impl CloudRepository {
    /// Read the named project; handles legacy project bootstrap metadata during cutover.
    pub async fn project(&self, id: &str) -> CloudResult<CloudProject> {
        crate::validate_identifier(id)?;
        let record = self.store.read(&project_key(id)).await?;
        decode_project(record)
    }
    /// Read all project namespaces in bounded pages for a verified administrator.
    pub async fn projects_page(
        &self,
        actor: &CloudUser,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudProject>> {
        administrator(actor)?;
        let records = self.store.list_projects(after, limit.min(100)).await?;
        records.into_iter().map(decode_project).collect()
    }
    /// Create or edit structural hierarchy; adapter serializes structural edits and rejects cycles.
    pub async fn save_project(
        &self,
        actor: &CloudUser,
        mut project: CloudProject,
    ) -> CloudResult<CloudProject> {
        administrator(actor)?;
        validate_project(&project)?;
        if let Some(parent) = &project.parent_project_id {
            let _ = self.project(parent).await?;
        }
        let expected = project.revision;
        if expected > 0 {
            let prior = self.project(&project.id).await?;
            if prior.revision != expected {
                return Err(CloudError::Conflict);
            }
            project.created_at = prior.created_at;
        } else {
            project.created_at = timestamp()?;
        }
        project.updated_at = timestamp()?;
        project.revision += 1;
        self.store
            .commit(CloudTransaction {
                entities: vec![mutation(
                    &actor.id,
                    project_key(&project.id),
                    expected,
                    "cloud.project.saved.v3",
                    &project,
                )?],
                ..Default::default()
            })
            .await?;
        Ok(project)
    }
    /// Project administrators may edit metadata within their own namespace; hierarchy moves require a global administrator.
    pub async fn update_project_metadata(
        &self,
        caller: &CloudCaller,
        mut project: CloudProject,
    ) -> CloudResult<CloudProject> {
        caller.require(CloudPermission::Administer)?;
        if project.id != caller.project_id() {
            return Err(CloudError::PermissionDenied);
        }
        let current = self.project(&project.id).await?;
        if project.revision != current.revision {
            return Err(CloudError::Conflict);
        }
        if project.parent_project_id != current.parent_project_id {
            return Err(CloudError::PermissionDenied);
        }
        validate_project(&project)?;
        let expected = project.revision;
        project.revision += 1;
        project.created_at = current.created_at;
        project.updated_at = timestamp()?;
        self.store
            .commit(CloudTransaction {
                entities: vec![mutation(
                    caller.subject(),
                    project_key(&project.id),
                    expected,
                    "cloud.project.metadata-updated.v3",
                    &project,
                )?],
                ..Default::default()
            })
            .await?;
        Ok(project)
    }
}
