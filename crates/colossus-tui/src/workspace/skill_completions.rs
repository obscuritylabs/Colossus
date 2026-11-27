//! Workspace-wide completion refresh, independent of per-session skill selection.

use super::*;

#[derive(Default)]
pub(super) struct SharedSkillCompletions {
    skills: Option<Vec<String>>,
    generation: u64,
    read: Option<(u64, tokio::task::JoinHandle<()>)>,
}

impl Drop for SharedSkillCompletions {
    fn drop(&mut self) {
        if let Some((_, task)) = self.read.take() {
            task.abort();
        }
    }
}

impl Workspace {
    pub(super) fn inherit_skill_completions(&self, state: &mut TuiState) {
        if let Some(skills) = &self.shared_skill_completions.skills {
            state.set_skill_completions(skills);
        }
    }

    pub(crate) fn refresh_skill_completions(&mut self, host: Arc<dyn InteractiveHost>) {
        let catalog = &mut self.shared_skill_completions;
        catalog.generation = catalog.generation.wrapping_add(1);
        self.start_skill_completion_read(host);
    }

    fn start_skill_completion_read(&mut self, host: Arc<dyn InteractiveHost>) {
        let catalog = &mut self.shared_skill_completions;
        if catalog.read.is_some() {
            return;
        }
        let generation = catalog.generation;
        let events = self.events.clone();
        let task = tokio::spawn(async move {
            let skills =
                match tokio::time::timeout(Duration::from_secs(30), host.skill_completion_ids())
                    .await
                {
                    Ok(Ok(skills)) => skills,
                    _ => None,
                };
            let _ = events
                .send(WorkspaceEvent::SkillCompletions { generation, skills })
                .await;
        });
        catalog.read = Some((generation, task));
    }

    pub(super) fn receive_skill_completions(
        &mut self,
        generation: u64,
        skills: Option<Vec<String>>,
        host: Arc<dyn InteractiveHost>,
    ) {
        let catalog = &mut self.shared_skill_completions;
        if catalog.read.as_ref().map(|(current, _)| *current) != Some(generation) {
            return;
        }
        catalog.read = None;
        if generation != catalog.generation {
            // A newer live refresh was requested while this read was pending.
            self.start_skill_completion_read(host);
            return;
        }
        if let Some(skills) = skills {
            for state in std::iter::once(&mut self.active).chain(self.background.values_mut()) {
                state.set_skill_completions(&skills);
            }
            catalog.skills = Some(skills);
        }
    }
}
