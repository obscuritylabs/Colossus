//! Read-only reconciliation of Desktop and Control Plane runs in one selected runtime.
use crate::{run_bindings::RunBindings, run_list::RunList};
use colossus_sdk::{
    AgentRunClient, ApiError, ApiErrorCode, ApiErrorReason, Colossus, GetRunRequest,
    GetRunResponse, ListRunsRequest, Run,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Write},
    sync::{Arc, Mutex, PoisonError},
};

#[path = "conversation_runs/discovery_proof.rs"]
mod discovery_proof;
pub(crate) use discovery_proof::ConversationProofs;
use discovery_proof::{DiscoveryPage, DiscoveryProof};

const PREFIX: &str = "desktop-conversations-v2:";
// Match the public run discovery ceiling; each owner returns at most three runs.
const MAX_PAGE_SIZE: u32 = 3;
const MAX_CURSORS: usize = 32;
const MAX_CURSOR_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone)]
pub(crate) struct ConversationRuns {
    sources: Arc<Mutex<RunBindings<SourceBinding>>>,
    desktop_sessions: Arc<Mutex<RunBindings<()>>>,
    cursors: Arc<Mutex<CursorCache>>,
    proof_scope: Arc<Mutex<String>>,
    head_bindings: Arc<Mutex<HashMap<String, SourceBinding>>>,
}
impl Default for ConversationRuns {
    fn default() -> Self {
        Self {
            sources: Arc::default(),
            desktop_sessions: Arc::default(),
            cursors: Arc::default(),
            proof_scope: Arc::new(Mutex::new(uuid::Uuid::new_v4().to_string())),
            head_bindings: Arc::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConversationSource {
    Primary,
    Cloud,
    PrimarySharedReadOnly,
}
impl ConversationSource {
    pub(crate) fn uses_connector(self) -> bool {
        self == Self::Cloud
    }
    pub(crate) fn can_control(self) -> bool {
        self == Self::Primary
    }
}
#[derive(Clone)]
struct SourceBinding {
    source: ConversationSource,
    session_id: String,
}
pub(crate) struct VerifiedConversation {
    pub(crate) response: GetRunResponse,
    pub(crate) reader: Arc<dyn AgentRunClient>,
    pub(crate) source: ConversationSource,
    pub(crate) continuable: bool,
    pub(crate) discovery_proof: Option<String>,
}
pub(crate) struct ConversationPage {
    pub(crate) runs: Vec<Run>,
    pub(crate) sources: HashMap<String, ConversationSource>,
    pub(crate) proofs: HashMap<String, String>,
    pub(crate) next_page_token: Option<String>,
}
#[derive(Clone, Default, PartialEq, Serialize)]
struct SourcePage {
    token: Option<String>,
    done: bool,
    pending: VecDeque<Run>,
    proof: Option<DiscoveryPage>,
}
#[derive(Clone, Default, PartialEq, Serialize)]
struct Cursor {
    scope: String,
    primary: SourcePage,
    cloud: SourcePage,
}
#[derive(Default)]
struct CursorCache {
    entries: HashMap<String, (Cursor, usize)>,
    order: VecDeque<String>,
    bytes: usize,
}
struct CursorSize(usize);
impl Write for CursorSize {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_CURSOR_BYTES)
            .ok_or_else(|| io::Error::other("conversation cursor capacity exceeded"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl CursorCache {
    fn insert(&mut self, cursor: Cursor) -> Result<String, ApiError> {
        // Periodic first-page reconciliation must not evict an unchanged
        // foreground continuation just by issuing the same discovery read.
        if let Some(token) = self
            .entries
            .iter()
            .find_map(|(token, entry)| (entry.0 == cursor).then(|| token.clone()))
        {
            self.order.retain(|key| key != &token);
            self.order.push_back(token.clone());
            return Ok(token);
        }
        let mut size = CursorSize(0);
        serde_json::to_writer(&mut size, &cursor).map_err(|_| {
            ApiError::resource_exhausted(
                ApiErrorReason::CapacityExceeded,
                "Reload conversations before loading more history.",
            )
        })?;
        while self.entries.len() >= MAX_CURSORS || self.bytes + size.0 > MAX_CURSOR_BYTES {
            let key = self.order.pop_front().ok_or_else(invalid)?;
            if let Some((_, bytes)) = self.entries.remove(&key) {
                self.bytes -= bytes;
            }
        }
        let token = format!("{PREFIX}{}", uuid::Uuid::new_v4());
        self.bytes += size.0;
        self.order.push_back(token.clone());
        self.entries.insert(token.clone(), (cursor, size.0));
        Ok(token)
    }
}
fn set_token(request: &mut ListRunsRequest, token: Option<String>) {
    request
        .page
        .get_or_insert_with(|| colossus_sdk::PageRequest {
            page_size: 0,
            page_token: String::new(),
        })
        .page_token = token.unwrap_or_default();
}
fn invalid() -> ApiError {
    ApiError::invalid(
        ApiErrorReason::InvalidArgument,
        "page_token",
        "The conversation page is no longer valid.",
    )
}
impl ConversationRuns {
    fn use_instance_scope(&self, instance: Option<String>) {
        if let Some(instance) = instance {
            *self
                .proof_scope
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = instance;
        }
    }
    fn binding(&self, run_id: &str) -> Option<SourceBinding> {
        self.sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .cloned()
            .or_else(|| {
                self.head_bindings
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(run_id)
                    .cloned()
            })
    }
    pub(crate) fn source(&self, run_id: &str) -> Option<ConversationSource> {
        self.binding(run_id).map(|binding| binding.source)
    }
    pub(crate) fn can_continue(&self, session_id: &str) -> bool {
        self.desktop_sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .is_some()
            || self
                .head_bindings
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .values()
                .any(|binding| binding.source.can_control() && binding.session_id == session_id)
    }
    fn remember(&self, runs: &[Run], cloud: bool) -> Result<(), ApiError> {
        let source = if cloud {
            ConversationSource::Cloud
        } else {
            ConversationSource::Primary
        };
        self.remember_source(runs, source)
    }
    fn remember_source(&self, runs: &[Run], source: ConversationSource) -> Result<(), ApiError> {
        // Snapshot the fixed foreground heads before taking the LRU/session
        // locks. Evicting ordinary metadata must not hide a conflicting owner.
        let heads = self
            .head_bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut sources = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
        let conflicts = |existing: &SourceBinding, run: &Run| {
            existing.session_id != run.session_id
                || (existing.source != source
                    && existing.source != ConversationSource::PrimarySharedReadOnly)
        };
        if runs.iter().any(|run| {
            sources
                .get(&run.run_id)
                .is_some_and(|existing| conflicts(existing, run))
                || heads
                    .get(&run.run_id)
                    .is_some_and(|existing| conflicts(existing, run))
        }) {
            return Err(source_changed());
        }
        let mut sessions = self
            .desktop_sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for run in runs {
            sources.insert(
                run.run_id.clone(),
                SourceBinding {
                    source,
                    session_id: run.session_id.clone(),
                },
            );
            if source.can_control() {
                sessions.insert(run.session_id.clone(), ());
            }
        }
        Ok(())
    }
    pub(crate) fn remember_primary(&self, run: &Run) -> Result<(), ApiError> {
        self.remember(std::slice::from_ref(run), false)
    }
    pub(crate) fn reader(
        client: &Colossus,
        source: ConversationSource,
    ) -> Result<Arc<dyn AgentRunClient>, ApiError> {
        reader_for(source, client.agent_runs(), client.connector_runs())
    }
    pub(crate) async fn get(
        &self,
        client: &Colossus,
        list: &RunList,
        request: GetRunRequest,
    ) -> Result<VerifiedConversation, ApiError> {
        self.use_instance_scope(client.instance_id().map(|id| id.to_string()));
        self.get_readers(client.agent_runs(), client.connector_runs(), list, request)
            .await
    }
    pub(crate) async fn get_with_proofs(
        &self,
        client: &Colossus,
        list: &RunList,
        request: GetRunRequest,
        proofs: ConversationProofs,
    ) -> Result<VerifiedConversation, ApiError> {
        self.use_instance_scope(client.instance_id().map(|id| id.to_string()));
        self.get_readers_with_proofs(
            client.agent_runs(),
            client.connector_runs(),
            list,
            request,
            proofs,
        )
        .await
    }
    async fn get_readers(
        &self,
        primary: Arc<dyn AgentRunClient>,
        cloud: Option<Arc<dyn AgentRunClient>>,
        list: &RunList,
        request: GetRunRequest,
    ) -> Result<VerifiedConversation, ApiError> {
        self.get_readers_with_proofs(primary, cloud, list, request, ConversationProofs::default())
            .await
    }
    async fn get_readers_with_proofs(
        &self,
        primary: Arc<dyn AgentRunClient>,
        cloud: Option<Arc<dyn AgentRunClient>>,
        list: &RunList,
        request: GetRunRequest,
        proofs: ConversationProofs,
    ) -> Result<VerifiedConversation, ApiError> {
        let proof_scope = self
            .proof_scope
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let discovery = proofs
            .discovery
            .as_deref()
            .map(|value| DiscoveryProof::decode(value, &proof_scope))
            .transpose()?;
        let (discovery_binding, accepted_discovery) = self
            .revalidate_discovery(
                primary.clone(),
                cloud.clone(),
                list,
                &request.run_id,
                discovery.as_ref(),
            )
            .await?;
        // A cache miss is not primary authority. Only a typed absence allows
        // trying the independent connector grant; denial and transport errors stop.
        let binding = discovery_binding.or_else(|| self.binding(&request.run_id));
        let known = binding.as_ref().map(|binding| binding.source);
        let (mut source, reader, response) = if let Some(source) = known {
            let reader = reader_for(source, primary.clone(), cloud.clone())?;
            let response = reader.get_run(request.clone()).await?;
            (source, reader, response)
        } else {
            match primary.get_run(request.clone()).await {
                Ok(response) => (ConversationSource::Primary, primary.clone(), response),
                Err(error)
                    if error.code == ApiErrorCode::NotFound
                        && error.reason == ApiErrorReason::RunNotFound =>
                {
                    let Some(reader) = cloud.clone() else {
                        return Err(error);
                    };
                    let response = reader.get_run(request.clone()).await?;
                    (ConversationSource::Cloud, reader, response)
                }
                Err(error) => return Err(error),
            }
        };
        if response.run.run_id != request.run_id
            || response.run.session_id.is_empty()
            || response.run.session_id.len() > 128
            || binding
                .as_ref()
                .is_some_and(|binding| binding.session_id != response.run.session_id)
        {
            return Err(source_changed());
        }
        // GetRun also permits shared reads. Only owned ListRuns/CreateRun
        // provenance, or an exact owned-run discovery proof, grants control.
        if !source.uses_connector() && known != Some(ConversationSource::Primary) {
            source = if self
                .owned_proof(primary.as_ref(), list, &response.run, true)
                .await?
            {
                ConversationSource::Primary
            } else {
                ConversationSource::PrimarySharedReadOnly
            };
        }
        self.remember_source(std::slice::from_ref(&response.run), source)?;
        let continuation = proofs
            .continuation
            .as_deref()
            .map(|value| DiscoveryProof::decode(value, &proof_scope))
            .transpose()?;
        let continued = if let Some(proof) = &continuation {
            if proof.page.connector || proof.session_id != response.run.session_id {
                return Err(invalid());
            }
            if !source.can_control() && !self.can_continue(&response.run.session_id) {
                match self.replay_proof(primary.clone(), None, list, proof).await {
                    Ok(_) => true,
                    Err(error) if error.reason == ApiErrorReason::InternalInvariant => {
                        return Err(error);
                    }
                    Err(_) => false,
                }
            } else {
                true
            }
        } else {
            false
        };
        let continuable = source.can_control()
            || continued
            || self.can_continue(&response.run.session_id)
            || (source.uses_connector()
                && self
                    .owned_proof(primary.as_ref(), list, &response.run, false)
                    .await?);
        Ok(VerifiedConversation {
            response,
            reader,
            source,
            continuable,
            discovery_proof: proofs.discovery.filter(|_| accepted_discovery),
        })
    }
    async fn revalidate_discovery(
        &self,
        primary: Arc<dyn AgentRunClient>,
        cloud: Option<Arc<dyn AgentRunClient>>,
        list: &RunList,
        run_id: &str,
        proof: Option<&DiscoveryProof>,
    ) -> Result<(Option<SourceBinding>, bool), ApiError> {
        let Some(proof) = proof else {
            return Ok((None, false));
        };
        if proof.run_id != run_id {
            return Err(invalid());
        }
        let binding = self.binding(run_id);
        if let Some(binding) = &binding
            && (binding.session_id != proof.session_id
                || (binding.source != ConversationSource::PrimarySharedReadOnly
                    && binding.source != proof.source()))
        {
            return Err(source_changed());
        }
        if binding
            .as_ref()
            .is_some_and(|binding| binding.source != ConversationSource::PrimarySharedReadOnly)
        {
            return Ok((binding, true));
        }
        match self.replay_proof(primary, cloud, list, proof).await {
            Ok(binding) => Ok((Some(binding), true)),
            Err(error) if error.reason == ApiErrorReason::InternalInvariant => Err(error),
            // Optional stale receipts and proof outages cannot revoke an
            // authenticated read. No hint authority reaches the read below.
            Err(_) => Ok((None, false)),
        }
    }
    async fn owned_proof(
        &self,
        primary: &dyn AgentRunClient,
        list: &RunList,
        run: &Run,
        exact_run: bool,
    ) -> Result<bool, ApiError> {
        match self.probe_owned(primary, list, run, exact_run).await {
            Ok(proved) => Ok(proved),
            Err(error) if error.reason == ApiErrorReason::InternalInvariant => Err(error),
            // A failed optional ownership proof cannot grant control or remove
            // a read that GetRun already authenticated successfully.
            Err(_) => Ok(false),
        }
    }
    async fn probe_owned(
        &self,
        primary: &dyn AgentRunClient,
        list: &RunList,
        run: &Run,
        exact_run: bool,
    ) -> Result<bool, ApiError> {
        let session_id = &run.session_id;
        let mut token = String::new();
        // Filtered public discovery may yield empty advancing pages. Bound the
        // proof search; an incomplete scan grants no continuation authority.
        for _ in 0..16 {
            let page = list
                .list_client(
                    primary,
                    ListRunsRequest {
                        session_id: Some(session_id.to_owned()),
                        statuses: vec![],
                        include_archived: true,
                        page: Some(colossus_sdk::PageRequest {
                            page_size: if exact_run { MAX_PAGE_SIZE } else { 1 },
                            page_token: token.clone(),
                        }),
                    },
                )
                .await?;
            if page
                .runs
                .iter()
                .any(|run| run.session_id.as_str() != session_id.as_str())
            {
                return Err(source_changed());
            }
            if !page.runs.is_empty() {
                self.remember(&page.runs, false)?;
                if !exact_run || page.runs.iter().any(|owned| owned.run_id == run.run_id) {
                    return Ok(true);
                }
            }
            let next = page
                .page
                .map(|page| page.next_page_token)
                .filter(|next| !next.is_empty());
            let Some(next) = next else { return Ok(false) };
            if next == token {
                return Err(invalid());
            }
            token = next;
        }
        Ok(false)
    }
    pub(crate) async fn list(
        &self,
        client: &Colossus,
        list: &RunList,
        request: ListRunsRequest,
        history_replacement: bool,
    ) -> Result<ConversationPage, ApiError> {
        self.use_instance_scope(client.instance_id().map(|id| id.to_string()));
        self.list_readers_with_history(
            client
                .instance_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            client.agent_runs(),
            client.connector_runs(),
            list,
            request,
            history_replacement,
        )
        .await
    }

    #[cfg(test)]
    async fn list_readers(
        &self,
        instance: String,
        primary: Arc<dyn AgentRunClient>,
        cloud_reader: Option<Arc<dyn AgentRunClient>>,
        list: &RunList,
        request: ListRunsRequest,
    ) -> Result<ConversationPage, ApiError> {
        self.list_readers_with_history(instance, primary, cloud_reader, list, request, false)
            .await
    }
    async fn list_readers_with_history(
        &self,
        instance: String,
        primary: Arc<dyn AgentRunClient>,
        cloud_reader: Option<Arc<dyn AgentRunClient>>,
        list: &RunList,
        mut request: ListRunsRequest,
        history_replacement: bool,
    ) -> Result<ConversationPage, ApiError> {
        let history_replacement = history_replacement
            && request.session_id.is_none()
            && request.statuses.is_empty()
            && request
                .page
                .as_ref()
                .is_none_or(|page| page.page_token.is_empty());
        let mut staged_heads = HashMap::new();
        let page_size = request.page.as_ref().map_or(MAX_PAGE_SIZE, |page| {
            if page.page_size == 0 {
                MAX_PAGE_SIZE
            } else {
                page.page_size.min(MAX_PAGE_SIZE)
            }
        });
        let token = request
            .page
            .as_mut()
            .map(|page| std::mem::take(&mut page.page_token))
            .filter(|token| !token.is_empty());
        request.page = Some(colossus_sdk::PageRequest {
            page_size,
            page_token: String::new(),
        });
        let scope = hex::encode(Sha256::digest(
            serde_json::to_vec(&(instance, &request, cloud_reader.is_some()))
                .map_err(|_| invalid())?,
        ));
        // Pending rows stay in native custody. A renderer cannot forge a row or
        // source binding by editing a serialized continuation token.
        let mut cursor = self.restore_cursor(scope, token, cloud_reader.is_some())?;
        let mut runs = Vec::with_capacity(page_size as usize);
        let mut sources = HashMap::new();
        let mut proofs = HashMap::new();
        for _ in 0..page_size {
            if !self
                .refill(
                    &mut cursor.primary,
                    primary.as_ref(),
                    list,
                    &request,
                    false,
                    history_replacement.then_some(&mut staged_heads),
                )
                .await?
            {
                break;
            }
            if let Some(cloud) = cloud_reader.as_ref()
                && !self
                    .refill(
                        &mut cursor.cloud,
                        cloud.as_ref(),
                        list,
                        &request,
                        true,
                        history_replacement.then_some(&mut staged_heads),
                    )
                    .await?
            {
                break;
            }
            // Upstream cursors preserve run creation/discovery order, not live
            // update order. Keep that order and prefer Desktop for equal times.
            let (source, identity) =
                match (cursor.primary.pending.front(), cursor.cloud.pending.front()) {
                    (Some(left), Some(right)) if left.created_at >= right.created_at => {
                        (&mut cursor.primary, ConversationSource::Primary)
                    }
                    (_, Some(_)) => (&mut cursor.cloud, ConversationSource::Cloud),
                    (Some(_), None) => (&mut cursor.primary, ConversationSource::Primary),
                    (None, None) => break,
                };
            if let Some(run) = source.pending.pop_front() {
                // Replayed cursors can outlive their discovery metadata.
                self.remember(std::slice::from_ref(&run), identity.uses_connector())?;
                sources.insert(run.run_id.clone(), identity);
                if let Some(page) = &source.proof {
                    proofs.insert(run.run_id.clone(), DiscoveryProof::encode(page, &run)?);
                }
                runs.push(run);
            }
        }
        self.finish_page(
            cursor,
            runs,
            sources,
            proofs,
            history_replacement.then_some(staged_heads),
        )
    }

    fn restore_cursor(
        &self,
        scope: String,
        token: Option<String>,
        cloud_available: bool,
    ) -> Result<Cursor, ApiError> {
        if let Some(token) = token {
            let cache = self.cursors.lock().unwrap_or_else(PoisonError::into_inner);
            let cursor = cache
                .entries
                .get(&token)
                .map(|entry| entry.0.clone())
                .ok_or_else(invalid)?;
            if cursor.scope != scope {
                return Err(invalid());
            }
            Ok(cursor)
        } else {
            Ok(Cursor {
                scope,
                cloud: SourcePage {
                    done: !cloud_available,
                    ..SourcePage::default()
                },
                ..Cursor::default()
            })
        }
    }

    fn finish_page(
        &self,
        cursor: Cursor,
        runs: Vec<Run>,
        sources: HashMap<String, ConversationSource>,
        proofs: HashMap<String, String>,
        staged_heads: Option<HashMap<String, SourceBinding>>,
    ) -> Result<ConversationPage, ApiError> {
        let more = !cursor.primary.done
            || !cursor.cloud.done
            || !cursor.primary.pending.is_empty()
            || !cursor.cloud.pending.is_empty();
        let next_page_token = if more {
            Some(
                self.cursors
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(cursor)?,
            )
        } else {
            None
        };
        if let Some(staged_heads) = staged_heads {
            *self
                .head_bindings
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = staged_heads;
        }
        Ok(ConversationPage {
            runs,
            sources,
            proofs,
            next_page_token,
        })
    }

    async fn refill(
        &self,
        source: &mut SourcePage,
        reader: &dyn AgentRunClient,
        list: &RunList,
        request: &ListRunsRequest,
        cloud: bool,
        mut staged_heads: Option<&mut HashMap<String, SourceBinding>>,
    ) -> Result<bool, ApiError> {
        // Filtered upstream scans can legitimately produce empty pages. Bound
        // admission work and return a continuation without guessing their order.
        for _ in 0..4 {
            if !source.pending.is_empty() || source.done {
                return Ok(true);
            }
            let mut request = request.clone();
            set_token(&mut request, source.token.clone());
            let proof = DiscoveryPage {
                scope: self
                    .proof_scope
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
                connector: cloud,
                request: request.clone(),
            };
            let mut page = list.list_client(reader, request).await?;
            if page.runs.len() > MAX_PAGE_SIZE as usize {
                return Err(source_changed());
            }
            if let Some(head) = staged_heads.as_deref_mut()
                && proof
                    .request
                    .page
                    .as_ref()
                    .is_some_and(|page| page.page_token.is_empty())
            {
                let identity = if cloud {
                    ConversationSource::Cloud
                } else {
                    ConversationSource::Primary
                };
                if page.runs.iter().any(|run| {
                    head.get(&run.run_id).is_some_and(|binding| {
                        binding.source != identity || binding.session_id != run.session_id
                    })
                }) {
                    return Err(source_changed());
                }
                head.retain(|_, binding| binding.source != identity);
                head.extend(page.runs.iter().map(|run| {
                    (
                        run.run_id.clone(),
                        SourceBinding {
                            source: identity,
                            session_id: run.session_id.clone(),
                        },
                    )
                }));
            }
            source.proof = Some(proof);
            for run in &mut page.runs {
                // Desktop history retains result metadata; full released output
                // is fetched only when opening a run through GetRun.
                if let Some(colossus_sdk::RunTerminal::Result(result)) = &mut run.terminal {
                    result.output = String::new();
                }
            }
            let next = page
                .page
                .map(|page| page.next_page_token)
                .filter(|token| !token.is_empty());
            if next.is_some() && next == source.token {
                return Err(invalid());
            }
            self.remember(&page.runs, cloud)?;
            source.pending.extend(page.runs);
            source.token = next;
            source.done = source.token.is_none();
        }
        Ok(!source.pending.is_empty() || source.done)
    }
}

fn source_changed() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InternalInvariant,
        "Conversation source identity changed. Reload the selected workspace.",
    )
}
fn reader_for(
    source: ConversationSource,
    primary: Arc<dyn AgentRunClient>,
    cloud: Option<Arc<dyn AgentRunClient>>,
) -> Result<Arc<dyn AgentRunClient>, ApiError> {
    match source {
        ConversationSource::Primary | ConversationSource::PrimarySharedReadOnly => Ok(primary),
        ConversationSource::Cloud => cloud.ok_or_else(|| {
            ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "The Control Plane conversation reader is unavailable.",
            )
        }),
    }
}

/// Fetch only released user text for this exact discovered cloud run, never private worker data.
pub(crate) async fn initial_prompt(reader: &dyn AgentRunClient, run: &Run) -> Option<String> {
    let mut page_token = None;
    for _ in 0..16 {
        let page = reader
            .list_session_activity(colossus_sdk::ListSessionActivityRequest {
                source_run_id: run.run_id.clone(),
                query: String::new(),
                lanes: vec![colossus_sdk::SessionActivityLane::Agent],
                kinds: vec![colossus_sdk::SessionActivityKind::User],
                statuses: vec![],
                page: Some(colossus_sdk::PageRequest {
                    page_size: 100,
                    page_token: page_token.unwrap_or_default(),
                }),
            })
            .await
            .ok()?;
        for activity in page.activities {
            if activity.run_id.as_deref() == Some(run.run_id.as_str())
                && let Some(input) = activity.input
                && input.format == "text"
                && !input.value.is_empty()
            {
                return Some(input.value);
            }
        }
        page_token = page
            .page
            .map(|page| page.next_page_token)
            .filter(|token| !token.is_empty());
        if page_token.is_none() {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use colossus_sdk::*;
    struct Reader {
        id: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
        rows: Option<Vec<Run>>,
    }
    fn run(id: &str, session: &str) -> Run {
        Run {
            run_id: id.into(),
            session_id: session.into(),
            title: id.into(),
            title_revision: 0,
            role: "primary".into(),
            mode: RunMode::Execute,
            status: RunStatus::Completed,
            created_at: "2026-10-09T00:00:00Z".into(),
            updated_at: "2026-10-09T00:00:00Z".into(),
            started_at: None,
            finished_at: None,
            last_sequence: 1,
            pending_interaction_count: 0,
            terminal: None,
            etag: "etag".into(),
            archived: false,
            plugin_skill_ids: vec![],
        }
    }
    #[async_trait]
    impl AgentRunClient for Reader {
        async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
            let token = request
                .page
                .as_ref()
                .map_or("", |page| page.page_token.as_str());
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:{token}", self.id));
            if let Some(rows) = &self.rows {
                let start = if token.is_empty() {
                    0
                } else {
                    token.parse::<usize>().unwrap()
                };
                let size = request.page.as_ref().unwrap().page_size as usize;
                let end = (start + size).min(rows.len());
                return Ok(ListRunsResponse {
                    runs: rows[start..end].to_vec(),
                    page: (end < rows.len()).then(|| PageResponse {
                        next_page_token: end.to_string(),
                    }),
                });
            }
            Ok(ListRunsResponse {
                runs: vec![run(&format!("{}{}", self.id, token), "same-session")],
                page: (self.id == "cloud" && token.is_empty()).then(|| PageResponse {
                    next_page_token: "next".into(),
                }),
            })
        }
        async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
            panic!("read broker cannot create")
        }
        async fn get_run(&self, _: GetRunRequest) -> ApiResult<GetRunResponse> {
            panic!("not used")
        }
        async fn watch_run(&self, _: WatchRunRequest) -> ApiResult<RunUpdateStream> {
            panic!("not used")
        }
        async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
            panic!("read broker cannot cancel")
        }
        async fn respond_interaction(
            &self,
            _: RespondInteractionRequest,
        ) -> ApiResult<RespondInteractionResponse> {
            panic!("read broker cannot answer")
        }
    }
    fn request(token: Option<String>) -> ListRunsRequest {
        ListRunsRequest {
            session_id: None,
            statuses: vec![],
            include_archived: false,
            page: Some(PageRequest {
                page_size: 2,
                page_token: token.unwrap_or_default(),
            }),
        }
    }
    #[tokio::test]
    async fn pages_both_owners_without_repeating_a_finished_source_and_fences_queries() {
        let broker = ConversationRuns::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let primary: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "desktop",
            calls: calls.clone(),
            rows: None,
        });
        let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "cloud",
            calls: calls.clone(),
            rows: None,
        });
        let list = RunList::default();
        let first = broker
            .list_readers(
                "runtime-a".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                request(None),
            )
            .await
            .unwrap();
        assert_eq!(first.runs.len(), 2);
        assert!(broker.source("desktop") == Some(ConversationSource::Primary));
        assert!(broker.source("cloud") == Some(ConversationSource::Cloud));
        assert!(broker.can_continue("same-session"));
        let token = first.next_page_token.unwrap();
        assert!(
            broker
                .list_readers(
                    "runtime-b".into(),
                    primary.clone(),
                    Some(cloud.clone()),
                    &list,
                    request(Some(token.clone()))
                )
                .await
                .is_err()
        );
        let mut changed = request(Some(token.clone()));
        changed.session_id = Some("foreign".into());
        assert!(
            broker
                .list_readers(
                    "runtime-a".into(),
                    primary.clone(),
                    Some(cloud.clone()),
                    &list,
                    changed
                )
                .await
                .is_err()
        );
        let second = broker
            .list_readers(
                "runtime-a".into(),
                primary,
                Some(cloud),
                &list,
                request(Some(token)),
            )
            .await
            .unwrap();
        assert!(second.next_page_token.is_none());
        assert_eq!(second.runs[0].run_id, "cloudnext");
        assert_eq!(*calls.lock().unwrap(), ["desktop:", "cloud:", "cloud:next"]);
    }
    #[test]
    fn a_discovered_run_cannot_change_source_and_a_cloud_only_session_cannot_continue() {
        let broker = ConversationRuns::default();
        broker.remember(&[run("web", "cloud-only")], true).unwrap();
        assert!(!broker.can_continue("cloud-only"));
        assert!(broker.remember(&[run("web", "cloud-only")], false).is_err());
        assert!(broker.source("web") == Some(ConversationSource::Cloud));
    }
    fn rows(prefix: &str, newest: u32) -> Vec<Run> {
        (0..4)
            .map(|offset| {
                let mut run = run(&format!("{prefix}{offset}"), prefix);
                run.created_at = format!("2026-10-09T00:00:{:02}Z", newest - offset);
                run.updated_at = run.created_at.clone();
                run
            })
            .collect()
    }
    #[tokio::test]
    async fn merged_pages_emit_only_the_global_boundary_and_keep_unconsumed_rows_native() {
        let broker = ConversationRuns::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let primary: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "desktop",
            calls: calls.clone(),
            rows: Some(rows("local", 20)),
        });
        let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "cloud",
            calls: calls.clone(),
            rows: Some(rows("remote", 10)),
        });
        let list = RunList::default();
        let mut query = request(None);
        query.page.as_mut().unwrap().page_size = 3;
        let first = broker
            .list_readers(
                "runtime".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                query.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            first
                .runs
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            ["local0", "local1", "local2"]
        );
        let token = first.next_page_token.unwrap();
        assert!(!token.contains("local") && !token.contains("remote"));
        let mut forged = query.clone();
        forged.page.as_mut().unwrap().page_token = format!("{PREFIX}forged-rows");
        assert!(
            broker
                .list_readers(
                    "runtime".into(),
                    primary.clone(),
                    Some(cloud.clone()),
                    &list,
                    forged
                )
                .await
                .is_err()
        );
        let mut second_query = query.clone();
        second_query.page.as_mut().unwrap().page_token = token;
        let second = broker
            .list_readers(
                "runtime".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                second_query.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            second
                .runs
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            ["local3", "remote0", "remote1"]
        );
        let replay = broker
            .list_readers(
                "runtime".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                second_query,
            )
            .await
            .unwrap();
        assert_eq!(replay.runs, second.runs);
        query.page.as_mut().unwrap().page_token = second.next_page_token.unwrap();
        let final_page = broker
            .list_readers("runtime".into(), primary, Some(cloud), &list, query)
            .await
            .unwrap();
        assert_eq!(
            final_page
                .runs
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            ["remote2", "remote3"]
        );
        assert!(final_page.next_page_token.is_none());
        assert!(broker.source("remote3") == Some(ConversationSource::Cloud));
        assert!(!broker.can_continue("remote"));
    }

    #[test]
    fn native_cursor_custody_is_bounded_and_expired_handles_are_removed() {
        let mut cache = CursorCache::default();
        let first = cache.insert(Cursor::default()).unwrap();
        assert_eq!(cache.insert(Cursor::default()).unwrap(), first);
        for index in 0..MAX_CURSORS {
            cache
                .insert(Cursor {
                    scope: index.to_string(),
                    ..Cursor::default()
                })
                .unwrap();
        }
        assert_eq!(cache.entries.len(), MAX_CURSORS);
        assert!(!cache.entries.contains_key(&first));
        let mut oversized = Cursor::default();
        let mut run = run("large", "session");
        run.title = "x".repeat(MAX_CURSOR_BYTES + 1);
        oversized.primary.pending.push_back(run);
        assert!(cache.insert(oversized).is_err());
    }
}

#[cfg(test)]
#[path = "conversation_runs/revalidation_tests.rs"]
mod revalidation_tests;
