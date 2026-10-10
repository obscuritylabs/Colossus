use crate::{communication::RuntimeCommunicationApi, task_inputs::*};
use colossus_api::*;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    head: u64,
    after: Option<(String, String)>,
    binding: String,
}

fn snapshot(
    run: Run,
    status_updated_at: String,
    history: Vec<AgentTaskInput>,
) -> AgentTaskSnapshot {
    AgentTaskSnapshot {
        task_id: run.id,
        context_id: run.session_id,
        status: run.status,
        status_updated_at,
        last_sequence: run.last_sequence,
        output: run.result.map(|result| result.output),
        failure: run.failure.map(|failure| AgentTaskFailure {
            code: failure.code,
            message: failure.message,
            outcome_unknown: failure.outcome == OutcomeCertainty::Unknown,
        }),
        waiting_kind: run.pending_interaction.map(|interaction| {
            match interaction.kind {
                InteractionKind::Approval => "approval",
                InteractionKind::Prompt => "user_prompt",
            }
            .into()
        }),
        history,
    }
}

impl RuntimeCommunicationApi {
    pub(crate) fn task(
        &self,
        caller: &CallerContext,
        request: GetAgentTaskRequest,
    ) -> ApiResult<AgentTaskSnapshot> {
        caller.require_scope(scopes::AGENT_MESSAGES_READ)?;
        caller.require_scope(scopes::RUNS_READ)?;
        if !token(&request.task_id) || request.history_length > 16 {
            return Err(invalid());
        }
        let journal = self.runtime.journal();
        let stream = history_stream(&request.task_id);
        read_claim(journal.as_ref(), caller, &stream)?.ok_or_else(missing)?;
        let head = journal
            .head()
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?
            .0;
        let (run, status_at) = self
            .repository
            .task_snapshot(caller, &request.task_id, head)?
            .ok_or_else(missing)?;
        let history = history_at(
            journal.as_ref(),
            caller,
            &run.id,
            &run.session_id,
            request.history_length,
            head,
        )?;
        Ok(snapshot(run, status_at, history))
    }

    pub(crate) fn tasks(
        &self,
        caller: &CallerContext,
        request: ListAgentTasksRequest,
    ) -> ApiResult<ListAgentTasksResponse> {
        caller.require_scope(scopes::AGENT_MESSAGES_READ)?;
        caller.require_scope(scopes::RUNS_READ)?;
        let _permit = self
            .lists
            .acquire(caller.principal().application_id())
            .map_err(|_| capacity())?;
        if !(1..=20).contains(&request.page_size)
            || request.history_length > 16
            || request.statuses.len() > 9
            || request.context_id.as_ref().is_some_and(|id| !token(id))
        {
            return Err(invalid());
        }
        let lower = request
            .status_updated_after
            .as_deref()
            .map(|value| OffsetDateTime::parse(value, &Rfc3339).map_err(|_| invalid()))
            .transpose()?;
        let mut statuses = request.statuses.clone();
        statuses.sort();
        statuses.dedup();
        if statuses.len() != request.statuses.len() {
            return Err(invalid());
        }
        let binding = digest(
            &serde_json::to_vec(&(
                owner_key(caller),
                &request.context_id,
                &statuses,
                &request.status_updated_after,
                request.page_size,
                request.include_output,
                request.history_length,
            ))
            .map_err(|_| invalid())?,
        );
        let journal = self.runtime.journal();
        let current_head = journal
            .head()
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?
            .0;
        let cursor = if let Some(raw) = &request.page_token {
            if raw.len() > 512 {
                return Err(invalid());
            }
            let bytes = hex::decode(raw).map_err(|_| invalid())?;
            let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if cursor.binding != binding || cursor.head > current_head {
                return Err(invalid());
            }
            cursor
        } else {
            Cursor {
                head: current_head,
                after: None,
                binding,
            }
        };
        // Scan canonical allocation entries at a frozen head, retaining only this
        // page's best candidates. A query budget limits work, not task creation.
        let after = cursor
            .after
            .as_ref()
            .map(|(timestamp, id)| {
                if !token(id) {
                    return Err(invalid());
                }
                OffsetDateTime::parse(timestamp, &Rfc3339)
                    .map(|instant| (instant, id.as_str()))
                    .map_err(|_| invalid())
            })
            .transpose()?;
        let mut tasks = Vec::new();
        let mut total_size = 0_u32;
        let mut candidates = 0_u32;
        let mut scanned = 0_usize;
        let mut index_after = 0;
        'index: loop {
            let entries = journal
                .read_stream_from(&index_stream(caller), index_after, 64)
                .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
            if entries.is_empty() {
                break;
            }
            for event in &entries {
                if event.global_sequence > cursor.head {
                    break 'index;
                }
                index_after = event.stream_version;
                scanned += 1;
                if scanned > 4096 {
                    return Err(capacity());
                }
                let claim = decode_claim(journal.as_ref(), event, caller)?;
                let (run, status_at) = self
                    .repository
                    .task_snapshot(caller, &claim.input.task_id, cursor.head)?
                    .ok_or_else(missing)?;
                if request
                    .context_id
                    .as_ref()
                    .is_some_and(|id| id != &run.session_id)
                    || !statuses.is_empty() && !statuses.contains(&run.status)
                {
                    continue;
                }
                let instant = OffsetDateTime::parse(&status_at, &Rfc3339).map_err(|_| {
                    ApiError::failed_precondition(
                        ApiErrorReason::InternalInvariant,
                        "invalid task lifecycle time",
                    )
                })?;
                if lower.is_some_and(|lower| instant < lower) {
                    continue;
                }
                total_size += 1;
                if after.is_some_and(|(after_at, id)| {
                    instant > after_at || instant == after_at && run.id.as_str() >= id
                }) {
                    continue;
                }
                candidates += 1;
                let mut view = snapshot(run, status_at, Vec::new());
                if !request.include_output {
                    view.output = None;
                }
                tasks.push((instant, view));
                tasks.sort_by(|(left_at, left), (right_at, right)| {
                    right_at
                        .cmp(left_at)
                        .then_with(|| right.task_id.cmp(&left.task_id))
                });
                tasks.truncate(request.page_size as usize);
            }
        }
        let next_page_token = if candidates > request.page_size {
            let last = &tasks.last().ok_or_else(invalid)?.1;
            Some(hex::encode(
                serde_json::to_vec(&Cursor {
                    after: Some((last.status_updated_at.clone(), last.task_id.clone())),
                    ..cursor
                })
                .map_err(|_| invalid())?,
            ))
        } else {
            None
        };
        let mut page = tasks.into_iter().map(|(_, task)| task).collect::<Vec<_>>();
        let mut released_bytes = 0_usize;
        for task in &mut page {
            task.history = history_at(
                journal.as_ref(),
                caller,
                &task.task_id,
                &task.context_id,
                request.history_length,
                cursor.head,
            )?;
            released_bytes = released_bytes
                .saturating_add(task.output.as_ref().map_or(0, String::len))
                .saturating_add(
                    task.history
                        .iter()
                        .map(|input| input.text.len())
                        .sum::<usize>(),
                );
            if released_bytes > 1024 * 1024 {
                return Err(capacity());
            }
        }
        Ok(ListAgentTasksResponse {
            tasks: page,
            next_page_token,
            total_size,
        })
    }
}

fn invalid() -> ApiError {
    ApiError::invalid(
        ApiErrorReason::InvalidArgument,
        "query",
        "invalid agent task query or cursor",
    )
}

// Recent input at the same frozen journal head as the run snapshot. Walk bounded
// chunks to skip later arrivals instead of silently mixing two views of a task.
fn history_at(
    journal: &dyn colossus_ports::EventJournal,
    caller: &CallerContext,
    task: &str,
    context: &str,
    limit: u32,
    head: u64,
) -> ApiResult<Vec<AgentTaskInput>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut before = None;
    let mut history = Vec::new();
    let mut scanned = 0;
    loop {
        let page = journal
            .read_stream_backwards(&history_stream(task), before, 32)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
        if page.is_empty() {
            break;
        }
        before = page.last().map(|event| event.stream_version);
        for event in &page {
            scanned += 1;
            if scanned > 4096 {
                return Err(capacity());
            }
            if event.global_sequence > head {
                continue;
            }
            let input = decode_claim(journal, event, caller)?.input;
            if input.task_id != task || input.context_id != context {
                return Err(missing());
            }
            history.push(input);
            if history.len() == limit as usize {
                history.reverse();
                return Ok(history);
            }
        }
    }
    history.reverse();
    Ok(history)
}
