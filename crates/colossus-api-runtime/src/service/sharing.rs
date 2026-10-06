//! Runtime-owned disclosure relationships; receiving applications retain their own grants.
use super::*;
use colossus_api::{
    ListVisibleRunsResponse, SetWorkspaceSharingRequest, VisibleRun, WorkspaceSharingState,
};
use colossus_contracts::{EventClassification, ExecutionContext, NewEvent};
use serde::{Deserialize, Serialize};

const SHARE_EVENT: &str = "api.workspace.sharing.changed.v1";
const MAX_SHARE_EVENTS: usize = 4096;
const MAX_SHARED_SOURCES: usize = 16;
const CURSOR_PREFIX: &str = "visible1:";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareRecord {
    owner: String,
    owner_kind: colossus_api::ApplicationKind,
    recipient: String,
    enabled: bool,
    allow_continuation: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VisibleCursor {
    source: usize,
    token: Option<String>,
    digest: String,
}

fn share_stream(recipient: &str) -> String {
    format!(
        "api.workspace.sharing:{}",
        hex::encode(Sha256::digest(recipient.as_bytes()))
    )
}

impl RuntimeAgentRunApi {
    fn shares(&self, recipient: &CallerContext) -> ApiResult<(Vec<ShareRecord>, u64)> {
        recipient.require_scope(scopes::RUNS_READ)?;
        let journal = self.runtime.journal();
        let mut records = BTreeMap::new();
        let mut cursor = 0;
        let mut count = 0;
        loop {
            let events = journal
                .read_stream_from(
                    &share_stream(recipient.principal().application_id()),
                    cursor,
                    128,
                )
                .map_err(|error| ApiError::from_store(&error, recipient.request_id()))?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                count += 1;
                if count > MAX_SHARE_EVENTS {
                    return Err(capacity_error(recipient));
                }
                let record: ShareRecord = serde_json::from_value(
                    journal
                        .decrypt_payload(event)
                        .map_err(|error| ApiError::from_store(&error, recipient.request_id()))?,
                )
                .map_err(|_| recovery_invariant(recipient))?;
                if event.event_type != SHARE_EVENT
                    || event.actor.actor_type != ActorType::Application
                    || event.actor.id != record.owner
                    || record.recipient != recipient.principal().application_id()
                    || event.stream_version != cursor + 1
                    || (record.allow_continuation && !record.enabled)
                {
                    return Err(recovery_invariant(recipient));
                }
                cursor = event.stream_version;
                records.insert(record.owner.clone(), record.clone());
            }
            if events.len() < 128 {
                break;
            }
        }
        let records = records
            .into_values()
            .filter(|record| record.enabled)
            .collect::<Vec<_>>();
        if records.len() > MAX_SHARED_SOURCES {
            return Err(capacity_error(recipient));
        }
        Ok((records, cursor))
    }

    fn recipient_context(
        &self,
        caller: &CallerContext,
        application: &str,
    ) -> ApiResult<CallerContext> {
        let principal = ApplicationPrincipal::authenticated(
            application,
            caller.principal().credential_id(),
            caller.principal().kind(),
            [colossus_api::ApiScope::new(scopes::RUNS_READ)?],
            [],
            [],
        )?;
        Ok(CallerContext::authenticated(
            principal,
            caller.request_id().clone(),
        ))
    }

    fn read_source_context(
        &self,
        recipient: &CallerContext,
        record: &ShareRecord,
    ) -> ApiResult<CallerContext> {
        // This is resource authorization only. It contains no execution, control,
        // interaction, role, or tool authority and cannot create a source-owned run.
        let principal = ApplicationPrincipal::authenticated(
            &record.owner,
            recipient.principal().credential_id(),
            record.owner_kind,
            [colossus_api::ApiScope::new(scopes::RUNS_READ)?],
            [],
            [],
        )?;
        Ok(CallerContext::authenticated(
            principal,
            recipient.request_id().clone(),
        ))
    }

    pub(super) fn persist_workspace_sharing(
        &self,
        caller: &CallerContext,
        request: SetWorkspaceSharingRequest,
    ) -> ApiResult<WorkspaceSharingState> {
        caller.require_scope(scopes::RUNS_CONTROL)?;
        caller.require_scope(scopes::RUNS_READ)?;
        if request.allow_continuation {
            caller.require_scope(scopes::RUNS_EXECUTE)?;
        }
        if request.recipient_application_id == caller.principal().application_id()
            || (request.allow_continuation && !request.enabled)
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "recipient_application_id",
                "invalid workspace sharing relationship",
            ));
        }
        let recipient = self.recipient_context(caller, &request.recipient_application_id)?;
        let journal = self.runtime.journal();
        let desired = ShareRecord {
            owner: caller.principal().application_id().into(),
            owner_kind: caller.principal().kind(),
            recipient: request.recipient_application_id.clone(),
            enabled: request.enabled,
            allow_continuation: request.allow_continuation,
        };
        for _ in 0..32 {
            let (records, version) = self.shares(&recipient)?;
            if records.iter().any(|record| {
                record.owner == desired.owner
                    && record.enabled == desired.enabled
                    && record.allow_continuation == desired.allow_continuation
            }) {
                break;
            }
            if version >= MAX_SHARE_EVENTS as u64 {
                return Err(capacity_error(caller));
            }
            let event = NewEvent {
                stream_id: share_stream(&desired.recipient),
                expected_stream_version: version,
                event_type: SHARE_EVENT.into(),
                actor: caller.actor(),
                event_version: 1,
                classification: EventClassification::System,
                context: ExecutionContext {
                    correlation_id: caller.request_id().as_str().into(),
                    ..ExecutionContext::default()
                },
                payload: serde_json::to_value(&desired).map_err(|_| recovery_invariant(caller))?,
            };
            match journal.append(event) {
                Ok(_) => {
                    return Ok(WorkspaceSharingState {
                        recipient_application_id: request.recipient_application_id,
                        enabled: request.enabled,
                        allow_continuation: request.allow_continuation,
                    });
                }
                Err(StoreError::Conflict { .. }) => continue,
                Err(error) => return Err(ApiError::from_store(&error, caller.request_id())),
            }
        }
        let (records, _) = self.shares(&recipient)?;
        let actual = records.iter().find(|record| record.owner == desired.owner);
        if (desired.enabled
            && actual.is_none_or(|record| record.allow_continuation != desired.allow_continuation))
            || (!desired.enabled && actual.is_some())
        {
            return Err(capacity_error(caller));
        }
        Ok(WorkspaceSharingState {
            recipient_application_id: request.recipient_application_id,
            enabled: request.enabled,
            allow_continuation: request.allow_continuation,
        })
    }

    pub(super) fn workspace_share_allowed(
        &self,
        recipient: &CallerContext,
        owner: &str,
        continuation: bool,
    ) -> ApiResult<bool> {
        Ok(self
            .shares(recipient)?
            .0
            .iter()
            .any(|record| record.owner == owner && (!continuation || record.allow_continuation)))
    }

    pub(super) fn shared_session_continuable(
        &self,
        caller: &CallerContext,
        session_id: &str,
    ) -> ApiResult<bool> {
        let events = self
            .runtime
            .journal()
            .read_stream_from(&format!("session:{session_id}"), 0, 1)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        match events.first() {
            Some(event)
                if event.event_type == "session.created.v1"
                    && event.actor.actor_type == ActorType::Application =>
            {
                self.workspace_share_allowed(caller, &event.actor.id, true)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn session_history_visible(
        &self,
        caller: &CallerContext,
        session_id: &str,
    ) -> ApiResult<bool> {
        let events = self
            .runtime
            .journal()
            .read_stream_from(&format!("session:{session_id}"), 0, 1)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        match events.first() {
            Some(event)
                if event.event_type == "session.created.v1"
                    && event.actor.actor_type == ActorType::Application =>
            {
                if event.actor.id == caller.principal().application_id() {
                    Ok(true)
                } else {
                    self.workspace_share_allowed(caller, &event.actor.id, false)
                }
            }
            _ => Ok(false),
        }
    }

    pub(super) fn visible_run_caller(
        &self,
        caller: &CallerContext,
        run_id: &str,
    ) -> ApiResult<CallerContext> {
        caller.require_scope(scopes::RUNS_READ)?;
        if self.repository.get_run(caller, run_id)?.is_some() {
            return Ok(caller.clone());
        }
        for record in self.shares(caller)?.0 {
            let source = self.read_source_context(caller, &record)?;
            if self.repository.get_run(&source, run_id)?.is_some() {
                return Ok(source);
            }
        }
        Err(missing_run(caller))
    }

    pub(super) fn discover_visible_runs(
        &self,
        caller: &CallerContext,
        mut request: ListRunsRequest,
    ) -> ApiResult<ListVisibleRunsResponse> {
        let (shares, version) = self.shares(caller)?;
        let mut digest = Sha256::new();
        digest.update(caller.principal().application_id().as_bytes());
        digest.update(version.to_be_bytes());
        let mut query = request.clone();
        query.page_token = None;
        digest.update(serde_json::to_vec(&query).map_err(|_| recovery_invariant(caller))?);
        let digest = hex::encode(digest.finalize());
        let cursor = match request.page_token.take() {
            Some(token) => {
                let value = token
                    .strip_prefix(CURSOR_PREFIX)
                    .ok_or_else(|| invalid_activity_cursor(caller))?;
                let decoded = URL_SAFE_NO_PAD
                    .decode(value)
                    .map_err(|_| invalid_activity_cursor(caller))?;
                let cursor: VisibleCursor = serde_json::from_slice(&decoded)
                    .map_err(|_| invalid_activity_cursor(caller))?;
                if cursor.digest != digest || cursor.source > shares.len() {
                    return Err(invalid_activity_cursor(caller));
                }
                cursor
            }
            None => VisibleCursor {
                source: 0,
                token: None,
                digest: digest.clone(),
            },
        };
        let mut source_index = cursor.source;
        request.page_token = cursor.token;
        loop {
            let source = if source_index == 0 {
                caller.clone()
            } else {
                self.read_source_context(caller, &shares[source_index - 1])?
            };
            let page = self.repository.list_runs(&source, &request)?;
            let next = if let Some(token) = page.next_page_token {
                Some(VisibleCursor {
                    source: source_index,
                    token: Some(token),
                    digest: digest.clone(),
                })
            } else if source_index < shares.len() {
                Some(VisibleCursor {
                    source: source_index + 1,
                    token: None,
                    digest: digest.clone(),
                })
            } else {
                None
            };
            if page.runs.is_empty() && next.as_ref().is_some_and(|next| next.token.is_none()) {
                source_index += 1;
                request.page_token = None;
                continue;
            }
            let controllable =
                source_index == 0 && caller.principal().has_scope(scopes::RUNS_CONTROL);
            let runs = page
                .runs
                .into_iter()
                .map(|run| VisibleRun {
                    continuable: caller.principal().has_scope(scopes::RUNS_EXECUTE)
                        && self.session_id(caller, Some(&run.session_id)).is_ok(),
                    run,
                    controllable,
                })
                .collect();
            let next_page_token = next
                .map(|cursor| {
                    serde_json::to_vec(&cursor)
                        .map(|bytes| format!("{CURSOR_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)))
                })
                .transpose()
                .map_err(|_| recovery_invariant(caller))?;
            return Ok(ListVisibleRunsResponse {
                runs,
                next_page_token,
            });
        }
    }
}
