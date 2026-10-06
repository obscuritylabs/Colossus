//! Project names and structural hierarchy, independent of membership inheritance.
use super::accounts::{administrator, mutation, timestamp};
use super::*;
use crate::{CloudProject, CloudUser, project_key, validate_project};
fn decode_project(record: crate::storage::EntityRecord) -> CloudResult<CloudProject> {
    if let Ok(mut project) = serde_json::from_value::<CloudProject>(record.value.clone()) {
        if project.id != record.key.id {
            return Err(CloudError::Storage);
        }
        project.revision = record.revision;
        return Ok(project);
    }
    let legacy = record.value.as_object().ok_or(CloudError::Storage)?;
    let label = legacy
        .get("label")
        .and_then(serde_json::Value::as_str)
        .ok_or(CloudError::Storage)?;
    if legacy
        .keys()
        .any(|key| !matches!(key.as_str(), "project_id" | "label" | "revision"))
        || legacy.get("project_id").and_then(serde_json::Value::as_str)
            != Some(record.key.id.as_str())
        || record.key.project_id != record.key.id
        || record.key.parent_id.is_some()
        || label.trim().is_empty()
        || label.len() > 256
        || label.chars().any(char::is_control)
        || legacy
            .get("revision")
            .is_some_and(|value| value.as_u64().is_none_or(|revision| revision == 0))
    {
        return Err(CloudError::Storage);
    }
    Ok(CloudProject {
        id: record.key.id.clone(),
        name: label.into(),
        description: String::new(),
        parent_project_id: None,
        archived: false,
        revision: record.revision,
        created_at: String::new(),
        updated_at: String::new(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    fn record(value: serde_json::Value) -> crate::storage::EntityRecord {
        crate::storage::EntityRecord {
            key: project_key("project-a"),
            revision: 4,
            value,
            page_cursor: None,
        }
    }
    #[test]
    fn legacy_metadata_is_stable_and_bad_modern_state_never_becomes_legacy() {
        let first = decode_project(record(
            serde_json::json!({"project_id":"project-a","label":"Legacy","revision":4}),
        ))
        .unwrap();
        let second = decode_project(record(
            serde_json::json!({"project_id":"project-a","label":"Legacy","revision":4}),
        ))
        .unwrap();
        assert_eq!(first.created_at, second.created_at);
        assert!(first.created_at.is_empty());
        assert!(first.updated_at.is_empty());
        assert_eq!(decode_project(record(serde_json::json!({"id":"project-a","name":"Modern","archived":"invalid","label":"Legacy","project_id":"project-a"}))).unwrap_err(),CloudError::Storage);
        assert_eq!(
            decode_project(record(
                serde_json::json!({"project_id":"other-project","label":"Legacy"})
            ))
            .unwrap_err(),
            CloudError::Storage
        );
    }
}
