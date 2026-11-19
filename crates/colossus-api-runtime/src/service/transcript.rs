//! Canonical export disclosure with reauthorized, authenticated snapshot cursors.

use super::*;
use colossus_api::{ReadSessionTranscriptRequest, TranscriptActivity, TranscriptPage};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};

const CURSOR_PREFIX: &str = "transcript1:";
const CURSOR_LIFETIME_SECONDS: i64 = 30 * 60;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TranscriptCursor {
    application_id: String,
    source_run_id: String,
    session_id: String,
    sharing_revision: u64,
    expires_at: i64,
    activity: TranscriptActivity,
    private_cursor: String,
}

impl RuntimeAgentRunApi {
    pub(super) async fn transcript_page(
        &self,
        caller: &CallerContext,
        request: ReadSessionTranscriptRequest,
    ) -> ApiResult<TranscriptPage> {
        caller.require_scope(scopes::RUNS_READ)?;
        request.validate()?;
        // Run visibility and original session ownership are independent. In
        // particular, a recipient-owned continuation cannot bypass revocation.
        let source_caller = self.visible_run_caller(caller, &request.source_run_id)?;
        let source = self
            .repository
            .get_run(&source_caller, &request.source_run_id)?
            .ok_or_else(|| missing_run(caller))?;
        if !self.session_history_visible(caller, &source.session_id)? {
            return Err(missing_run(caller));
        }
        let sharing_revision = self.transcript_sharing_revision(caller)?;
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let key = self.transcript_signing_key(caller)?;
        let cursor = request
            .page_token
            .as_deref()
            .map(|token| decode_cursor(caller, token, &key))
            .transpose()?;
        if cursor.as_ref().is_some_and(|cursor| {
            cursor.application_id != caller.principal().application_id()
                || cursor.source_run_id != source.id
                || cursor.session_id != source.session_id
                || cursor.sharing_revision != sharing_revision
                || cursor.expires_at < now
        }) {
            return Err(invalid_transcript_cursor(caller));
        }
        let _permit = self
            .transcript_pages
            .acquire(caller.principal().application_id())
            .await
            .map_err(|AdmissionLimitReached| capacity_error(caller))?;
        // Admission may wait. Revalidate source, sharing and cursor expiry before
        // entering the canonical reader, even when the caller owns a continuation.
        let current_source_caller = self.visible_run_caller(caller, &request.source_run_id)?;
        let current_source = self
            .repository
            .get_run(&current_source_caller, &request.source_run_id)?
            .ok_or_else(|| missing_run(caller))?;
        if current_source.session_id != source.session_id
            || !self.session_history_visible(caller, &source.session_id)?
            || self.transcript_sharing_revision(caller)? != sharing_revision
        {
            return Err(missing_run(caller));
        }
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.expires_at < OffsetDateTime::now_utc().unix_timestamp())
        {
            return Err(invalid_transcript_cursor(caller));
        }
        let mut page = self
            .runtime
            .read_session_transcript(
                &source.session_id,
                cursor.as_ref().map(|cursor| cursor.private_cursor.as_str()),
                request.bounded_page_size(),
            )
            .map_err(|error| match &error {
                RuntimeError::Store(StoreError::Adapter(message))
                    if message == "canonical transcript message exceeds the export page bound" =>
                {
                    ApiError::bounded_resource_exhausted(
                        ApiErrorReason::CapacityExceeded,
                        "a retained transcript message exceeds the export transport bound",
                    )
                    .with_correlation_id(caller.request_id().clone())
                }
                _ => activity_runtime_error(caller, error),
            })?;
        let activity = cursor.as_ref().map_or_else(
            || {
                if lock(&self.execution)
                    .active
                    .values()
                    .any(|run| run.writer.session_id() == source.session_id)
                {
                    TranscriptActivity::Active
                } else if page.snapshot.activity != TranscriptActivity::Active
                    && !lock(&self.pending_recoveries).is_empty()
                {
                    // Recovery entries are run-addressed. Without replaying every
                    // pending run, this reader cannot prove this session idle.
                    TranscriptActivity::Unknown
                } else {
                    page.snapshot.activity
                }
            },
            |cursor| cursor.activity,
        );
        page.snapshot.activity = activity;
        // Recheck disclosure after canonical reads before publishing any bytes.
        self.visible_run_caller(caller, &request.source_run_id)?;
        if !self.session_history_visible(caller, &source.session_id)?
            || self.transcript_sharing_revision(caller)? != sharing_revision
        {
            return Err(missing_run(caller));
        }
        page.next_cursor = page
            .next_cursor
            .take()
            .map(|private_cursor| {
                encode_cursor(
                    caller,
                    &TranscriptCursor {
                        application_id: caller.principal().application_id().into(),
                        source_run_id: source.id.clone(),
                        session_id: source.session_id.clone(),
                        sharing_revision,
                        expires_at: cursor
                            .as_ref()
                            .map_or(now + CURSOR_LIFETIME_SECONDS, |cursor| cursor.expires_at),
                        activity,
                        private_cursor,
                    },
                    &key,
                )
            })
            .transpose()?;
        Ok(page)
    }

    #[cfg(test)]
    pub(crate) async fn occupy_transcript_reader_for_test(
        &self,
        application_id: &str,
    ) -> impl Drop {
        self.transcript_pages.acquire(application_id).await.unwrap()
    }

    #[cfg(test)]
    pub(crate) fn occupy_metadata_reader_for_test(&self, application_id: &str) -> impl Drop {
        self.lists.acquire(application_id).unwrap()
    }

    fn transcript_sharing_revision(&self, caller: &CallerContext) -> ApiResult<u64> {
        let stream = format!(
            "api.workspace.sharing:{}",
            hex::encode(Sha256::digest(
                caller.principal().application_id().as_bytes()
            ))
        );
        self.runtime
            .journal()
            .read_stream_backwards(&stream, None, 1)
            .map(|events| events.first().map_or(0, |event| event.stream_version))
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))
    }

    fn transcript_signing_key(&self, caller: &CallerContext) -> ApiResult<[u8; 32]> {
        let mut saved = lock(&self.transcript_key);
        if let Some(key) = *saved {
            return Ok(key);
        }
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).map_err(|_| {
            ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "transcript cursor authentication is unavailable",
            )
            .with_correlation_id(caller.request_id().clone())
        })?;
        *saved = Some(key);
        Ok(key)
    }
}

fn encode_cursor(
    caller: &CallerContext,
    cursor: &TranscriptCursor,
    key: &[u8; 32],
) -> ApiResult<String> {
    let bytes = serde_json::to_vec(cursor).map_err(|_| invalid_transcript_cursor(caller))?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).map_err(|_| invalid_transcript_cursor(caller))?;
    mac.update(&bytes);
    let token = format!(
        "{CURSOR_PREFIX}{}.{}",
        URL_SAFE_NO_PAD.encode(&bytes),
        hex::encode(mac.finalize().into_bytes())
    );
    if token.len() > colossus_api::MAX_TRANSCRIPT_CURSOR_BYTES {
        return Err(ApiError::bounded_resource_exhausted(
            ApiErrorReason::CapacityExceeded,
            "transcript cursor exceeds its transport bound",
        ));
    }
    Ok(token)
}

fn decode_cursor(
    caller: &CallerContext,
    token: &str,
    key: &[u8; 32],
) -> ApiResult<TranscriptCursor> {
    let (value, tag) = token
        .strip_prefix(CURSOR_PREFIX)
        .and_then(|value| value.split_once('.'))
        .ok_or_else(|| invalid_transcript_cursor(caller))?;
    if tag.len() != 64 {
        return Err(invalid_transcript_cursor(caller));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| invalid_transcript_cursor(caller))?;
    let tag = hex::decode(tag).map_err(|_| invalid_transcript_cursor(caller))?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).map_err(|_| invalid_transcript_cursor(caller))?;
    mac.update(&bytes);
    mac.verify_slice(&tag)
        .map_err(|_| invalid_transcript_cursor(caller))?;
    serde_json::from_slice(&bytes).map_err(|_| invalid_transcript_cursor(caller))
}

fn invalid_transcript_cursor(caller: &CallerContext) -> ApiError {
    ApiError::invalid(
        ApiErrorReason::InvalidArgument,
        "page_token",
        "the transcript cursor is invalid or expired",
    )
    .with_correlation_id(caller.request_id().clone())
}
