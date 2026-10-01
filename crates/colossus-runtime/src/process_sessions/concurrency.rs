//! Permit-time actor/run concurrency, independent of the workspace session capacity.
use super::*;

type Scope = (String, Option<String>);
#[derive(Default)]
pub(super) struct ConcurrencyScopes(StdMutex<BTreeMap<Scope, u32>>);

impl ConcurrencyScopes {
    pub(super) fn acquire(
        self: &Arc<Self>,
        request: &EffectRequest,
        maximum: u32,
    ) -> Result<ScopeLease, ExecutionError> {
        let scope = (
            serde_json::to_string(&request.actor)
                .map_err(|_| ExecutionError::Failed("invalid concurrency actor".into()))?,
            request.context.run_id.clone(),
        );
        let mut active = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = active.get(&scope).copied().unwrap_or(0);
        if count >= maximum {
            return Err(ExecutionError::Failed(
                "managed process exceeds actor/run policy concurrency limit".into(),
            ));
        }
        active.insert(scope.clone(), count + 1);
        Ok(ScopeLease {
            scopes: Arc::clone(self),
            scope,
        })
    }
}

pub(super) struct ScopeLease {
    scopes: Arc<ConcurrencyScopes>,
    scope: Scope,
}
impl Drop for ScopeLease {
    fn drop(&mut self) {
        let mut active = self
            .scopes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(count) = active.get_mut(&self.scope) {
            *count -= 1;
            if *count == 0 {
                active.remove(&self.scope);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actor_run_scopes_are_independent_and_release_their_capacity() {
        let scopes = Arc::new(ConcurrencyScopes::default());
        let mut first = effect_request(system_actor("a"), "shell.run", "shell", json!({}));
        first.context.run_id = Some("run-1".into());
        let lease = scopes.acquire(&first, 1).expect("first slot");
        assert!(scopes.acquire(&first, 1).is_err());
        let mut other_actor = first.clone();
        other_actor.actor = system_actor("b");
        let _other_actor = scopes.acquire(&other_actor, 1).expect("independent actor");
        let mut other_run = first.clone();
        other_run.context.run_id = Some("run-2".into());
        let _other_run = scopes.acquire(&other_run, 1).expect("independent run");
        let extra = scopes.acquire(&first, 2).expect("larger authorized limit");
        assert!(
            scopes.acquire(&first, 1).is_err(),
            "tighter policy remains authoritative"
        );
        drop(extra);
        drop(lease);
        let lease = scopes.acquire(&first, 1).expect("released capacity");
        drop(lease);
        assert_eq!(scopes.0.lock().expect("scopes").len(), 2);
    }
}
