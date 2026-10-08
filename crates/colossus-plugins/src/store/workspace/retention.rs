//! Bounded cache metadata; immutable lifecycle and run provenance stay in their journals.

use super::*;

const CACHE_STREAM: &str = "plugin-workspace-cache";
pub(super) const RECENT_PER_SOURCE: usize = 8;
const MAX_CACHED_SNAPSHOTS: usize = 256;
const MAX_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::store) struct WorkspaceCache {
    pub(in crate::store) entries: BTreeMap<String, CacheEntry>,
    current: BTreeMap<String, String>,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::store) struct CacheEntry {
    pub(in crate::store) name: String,
    path: String,
    bytes: u64,
    generation: u64,
}

impl EventSourcedPluginRepository {
    pub(in crate::store) fn workspace_cache(&self) -> Result<Option<WorkspaceCache>, StoreError> {
        let cache = self
            .journal
            .read_stream_backwards(CACHE_STREAM, None, 1)?
            .first()
            .map(|event| {
                serde_json::from_value::<WorkspaceCache>(self.journal.decrypt_payload(event)?)
                    .map_err(adapter)
            })
            .transpose()?;
        let Some(cache) = cache else {
            return Ok(None);
        };
        if cache.entries.len() > MAX_CACHED_SNAPSHOTS || cache.current.len() > MAX_WORKSPACE_PLUGINS
        {
            return Err(adapter(
                "workspace snapshot cache metadata exceeds its bound",
            ));
        }
        for digest in cache.entries.keys().chain(cache.current.values()) {
            validate_lease_digest(digest)?;
        }
        for entry in cache.entries.values() {
            validate_plugin_name(&entry.name)?;
        }
        Ok(Some(cache))
    }

    pub(super) fn append_workspace_cache(
        &self,
        cache: &WorkspaceCache,
        actor: Actor,
    ) -> Result<(), StoreError> {
        if self.workspace_cache()?.as_ref() == Some(cache) {
            return Ok(());
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: CACHE_STREAM.into(),
            expected_stream_version: self
                .journal
                .read_stream_backwards(CACHE_STREAM, None, 1)?
                .first()
                .map_or(0, |event| event.stream_version),
            classification: EventClassification::Domain,
            event_type: "plugin.workspace-cache.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: "workspace-plugins".into(),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(cache).map_err(adapter)?,
        })?;
        Ok(())
    }
}

impl PluginStore {
    pub(super) fn prepare_workspace_cache(
        &self,
        repository: &EventSourcedPluginRepository,
        candidate: &WorkspacePluginCandidate,
        recoverable: &BTreeSet<String>,
        actor: Actor,
    ) -> Result<WorkspaceCache, StoreError> {
        let previous = repository.workspace_cache()?;
        if previous.is_none() && !repository.list_plugins(1)?.is_empty() {
            return Err(adapter(
                "workspace snapshots require a separate store from globally installed plugins",
            ));
        }
        let old = previous.unwrap_or_default();
        let grants = repository.workspace_grants()?;
        let mut protected = self.live_snapshot_digests()?;
        protected.extend(recoverable.iter().cloned());
        for (path, grant) in &grants {
            if grant.enabled
                && let Some(digest) = old.current.get(path)
            {
                protected.insert(digest.clone());
            }
        }
        protected.insert(candidate.artifact.manifest_digest.clone());
        let cache = plan_cache(&old, candidate, &grants, &protected)?;
        // Free superseded bytes before publication. If publication fails, every
        // accepted source's previous current snapshot and every run pin survives.
        let mut trimmed = old.clone();
        trimmed
            .entries
            .retain(|digest, _| cache.entries.contains_key(digest));
        trimmed
            .current
            .retain(|_, digest| trimmed.entries.contains_key(digest));
        // The empty index marks this as a dedicated workspace store before any
        // publication, including a failed first capture or interrupted write.
        if trimmed != old || repository.workspace_cache()?.is_none() {
            repository.append_workspace_cache(&trimmed, actor)?;
        }
        // Also collect an unjournaled publication left by a failed write or crash.
        self.gc_locked(repository)?;
        Ok(cache)
    }
}

fn plan_cache(
    old: &WorkspaceCache,
    candidate: &WorkspacePluginCandidate,
    grants: &BTreeMap<String, WorkspacePluginGrant>,
    protected: &BTreeSet<String>,
) -> Result<WorkspaceCache, StoreError> {
    let mut cache = old.clone();
    let digest = &candidate.artifact.manifest_digest;
    if cache.current.get(&candidate.source.path) != Some(digest)
        || !cache.entries.contains_key(digest)
    {
        cache.generation = cache
            .generation
            .checked_add(1)
            .ok_or_else(|| adapter("workspace cache generation exhausted"))?;
        cache.entries.insert(
            digest.clone(),
            CacheEntry {
                name: candidate.source.name.clone(),
                path: candidate.source.path.clone(),
                bytes: candidate.cache_bytes,
                generation: cache.generation,
            },
        );
    }
    cache
        .current
        .insert(candidate.source.path.clone(), digest.clone());
    cache
        .current
        .retain(|path, _| grants.contains_key(path) || *path == candidate.source.path);
    let mut by_age = cache
        .entries
        .iter()
        .map(|(digest, entry)| (entry.generation, digest.clone()))
        .collect::<Vec<_>>();
    by_age.sort();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut bytes = 0_u64;
    for entry in cache.entries.values() {
        *counts.entry(entry.path.clone()).or_default() += 1;
        bytes = bytes.saturating_add(entry.bytes);
    }
    // Recent history is a soft limit when a running or recoverable job pins it;
    // the workspace-wide count and byte budgets always remain hard limits.
    for (_, digest) in by_age {
        let entry = &cache.entries[&digest];
        if (counts[&entry.path] > RECENT_PER_SOURCE
            || cache.entries.len() > MAX_CACHED_SNAPSHOTS
            || bytes > MAX_CACHE_BYTES)
            && !protected.contains(&digest)
        {
            bytes = bytes.saturating_sub(entry.bytes);
            *counts.get_mut(&entry.path).expect("counted source") -= 1;
            cache.entries.remove(&digest);
        }
    }
    if cache.entries.len() > MAX_CACHED_SNAPSHOTS || bytes > MAX_CACHE_BYTES {
        return Err(adapter(
            "workspace plugin cache is full of retained snapshots; complete pending or retryable jobs, or disable unused workspace sources, before capturing more edits",
        ));
    }
    cache
        .current
        .retain(|_, digest| cache.entries.contains_key(digest));
    Ok(cache)
}

#[cfg(test)]
mod tests;
