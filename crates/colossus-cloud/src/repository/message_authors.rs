use super::CloudRepository;
use crate::{
    CloudError, CloudMessage, CloudMessageAuthor, CloudResult, CloudTask, CloudThread, UserAccount,
};
use std::collections::{BTreeMap, BTreeSet};

impl CloudRepository {
    /// Enrich only this authorized message page. Profiles never enter canonical storage.
    pub(super) async fn message_authors(
        &self,
        thread: &CloudThread,
        tasks: &[CloudTask],
        messages: &[CloudMessage],
    ) -> CloudResult<BTreeMap<String, CloudMessageAuthor>> {
        // A detail page contains at most 100 retained messages plus 100 synthesized turns.
        if messages.len() > 200 {
            return Err(CloudError::ResourceExhausted);
        }
        let known_tasks = tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<BTreeSet<_>>();
        let account_subject = |task: &CloudTask| {
            (task.thread_id.as_deref() == Some(thread.thread_id.as_str())
                && task.project_id == thread.project_id
                && task.node_id == thread.node_id
                && task.subject != "runtime"
                && !task.source_read_only)
                .then(|| task.subject.clone())
        };
        let mut subjects = tasks
            .iter()
            .filter_map(|task| account_subject(task).map(|subject| (task.task_id.clone(), subject)))
            .collect::<BTreeMap<_, _>>();
        let task_ids = messages
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| &message.task_id)
            .collect::<BTreeSet<_>>();
        // Task and message cursors are independent; older messages may reference turns
        // outside the loaded task page. Resolve those exact project-scoped identities.
        for id in task_ids {
            if !known_tasks.contains(id.as_str()) {
                match self.task(&thread.project_id, id).await {
                    Ok(task) => {
                        if let Some(subject) = account_subject(&task) {
                            subjects.insert(id.clone(), subject);
                        }
                    }
                    Err(CloudError::NotFound) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let ids = subjects
            .values()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut profiles = BTreeMap::new();
        for batch in ids.chunks(100) {
            for record in self.store.user_accounts(batch).await? {
                let account = UserAccount::try_from(record.value)?;
                profiles.insert(
                    account.user.id.clone(),
                    CloudMessageAuthor {
                        user_id: account.user.id,
                        display_name: account.user.display_name,
                    },
                );
            }
        }
        Ok(messages
            .iter()
            .filter(|message| message.role == "user")
            .filter_map(|message| {
                let subject = subjects.get(&message.task_id)?;
                let profile = profiles.get(subject)?;
                Some((message.message_id.clone(), profile.clone()))
            })
            .collect())
    }
}
