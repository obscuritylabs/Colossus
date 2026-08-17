use std::collections::BTreeMap;

use redb::{Database, ReadableDatabase as _, ReadableTable as _, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::{
    desktop_dto::{SpaceSearchPageDto, SpaceSearchResultDto},
    desktop_settings::{DesktopSettings, SettingsStore},
    dto::{CommandErrorDto, RunDto, RunModeDto, RunStatusDto},
};

const THREAD_SUMMARIES: TableDefinition<&str, &[u8]> =
    TableDefinition::new("desktop_thread_summaries_v1");
const MAX_TITLE_BYTES: usize = 512;
const MAX_TIMESTAMP_BYTES: usize = 64;
const MAX_SEARCH_RESULTS: usize = 100;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThreadSummaryRecord {
    schema_version: u8,
    space_id: String,
    run_id: String,
    session_id: String,
    title: String,
    mode: String,
    status: String,
    updated_at: String,
    #[serde(default)]
    archived: bool,
}

pub(crate) fn index_runs(
    settings: &DesktopSettings,
    space_id: &str,
    runs: &[RunDto],
) -> Result<(), CommandErrorDto> {
    if settings.space(space_id).is_none() {
        return Ok(());
    }
    let database = open_database()?;
    let write = database.begin_write().map_err(|_| index_error())?;
    {
        let mut table = write
            .open_table(THREAD_SUMMARIES)
            .map_err(|_| index_error())?;
        for run in runs {
            if settings
                .asides
                .iter()
                .any(|aside| aside.space_id == space_id && aside.session_id == run.session_id)
            {
                let key = format!("{space_id}:{}", run.run_id);
                table.remove(key.as_str()).map_err(|_| index_error())?;
                continue;
            }
            let Some(record) = record_from_run(space_id, run) else {
                continue;
            };
            let key = format!("{}:{}", record.space_id, record.run_id);
            let bytes = serde_json::to_vec(&record).map_err(|_| index_error())?;
            table
                .insert(key.as_str(), bytes.as_slice())
                .map_err(|_| index_error())?;
        }
    }
    write.commit().map_err(|_| index_error())
}

pub(crate) fn set_thread_archived(
    settings: &DesktopSettings,
    space_id: &str,
    session_id: &str,
    archived: bool,
) -> Result<(), CommandErrorDto> {
    if settings.space(space_id).is_none() {
        return Ok(());
    }
    let database = open_database()?;
    let write = database.begin_write().map_err(|_| index_error())?;
    {
        let mut table = write
            .open_table(THREAD_SUMMARIES)
            .map_err(|_| index_error())?;
        let mut replacements = Vec::new();
        for entry in table.iter().map_err(|_| index_error())? {
            let (key, value) = entry.map_err(|_| index_error())?;
            let Ok(mut record) = serde_json::from_slice::<ThreadSummaryRecord>(value.value())
            else {
                continue;
            };
            if record.space_id == space_id && record.session_id == session_id {
                record.archived = archived;
                replacements.push((
                    key.value().to_owned(),
                    serde_json::to_vec(&record).map_err(|_| index_error())?,
                ));
            }
        }
        for (key, value) in replacements {
            table
                .insert(key.as_str(), value.as_slice())
                .map_err(|_| index_error())?;
        }
    }
    write.commit().map_err(|_| index_error())
}

pub(crate) fn search(
    settings: &DesktopSettings,
    query: &str,
    scope_space_id: Option<&str>,
    include_archived: bool,
    offset: usize,
    limit: usize,
) -> Result<SpaceSearchPageDto, CommandErrorDto> {
    let database = open_database()?;
    let read = database.begin_read().map_err(|_| index_error())?;
    let table = read
        .open_table(THREAD_SUMMARIES)
        .map_err(|_| index_error())?;
    let normalized = query.trim().to_lowercase();
    let mut results_by_thread = BTreeMap::<(String, String), SpaceSearchResultDto>::new();
    for entry in table.iter().map_err(|_| index_error())? {
        let (_, value) = entry.map_err(|_| index_error())?;
        let Ok(record) = serde_json::from_slice::<ThreadSummaryRecord>(value.value()) else {
            continue;
        };
        if !valid_record(&record) || scope_space_id.is_some_and(|scope| scope != record.space_id) {
            continue;
        }
        let Some(space) = settings.space(&record.space_id) else {
            continue;
        };
        if (space.archived || record.archived) && !include_archived {
            continue;
        }
        let matches = normalized.is_empty()
            || record.title.to_lowercase().contains(&normalized)
            || record.mode.contains(&normalized)
            || record.status.contains(&normalized)
            || space.display_name.to_lowercase().contains(&normalized);
        if !matches {
            continue;
        }
        let result = SpaceSearchResultDto {
            space_id: record.space_id.clone(),
            space_name: space.display_name.clone(),
            target_id: record.space_id,
            run_id: record.run_id,
            session_id: record.session_id,
            title: record.title,
            mode: record.mode,
            attention: attention_status(&record.status),
            status: record.status,
            updated_at: record.updated_at,
            archived: space.archived,
            thread_archived: record.archived,
        };
        insert_latest_thread_result(&mut results_by_thread, result);
    }
    let mut results = results_by_thread.into_values().collect::<Vec<_>>();
    results.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.space_name.cmp(&right.space_name))
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    let limit = limit.clamp(1, MAX_SEARCH_RESULTS);
    let total = results.len();
    let page = results.into_iter().skip(offset).take(limit).collect();
    let next = offset.saturating_add(limit);
    Ok(SpaceSearchPageDto {
        results: page,
        next_cursor: if next < total {
            next.to_string()
        } else {
            String::new()
        },
    })
}

fn insert_latest_thread_result(
    results: &mut BTreeMap<(String, String), SpaceSearchResultDto>,
    candidate: SpaceSearchResultDto,
) {
    let key = (candidate.space_id.clone(), candidate.session_id.clone());
    let replace = results.get(&key).is_none_or(|current| {
        (candidate.updated_at.as_str(), candidate.run_id.as_str())
            > (current.updated_at.as_str(), current.run_id.as_str())
    });
    if replace {
        results.insert(key, candidate);
    }
}

pub(crate) fn attention_counts(
    settings: &DesktopSettings,
) -> Result<BTreeMap<String, u32>, CommandErrorDto> {
    let database = open_database()?;
    let read = database.begin_read().map_err(|_| index_error())?;
    let table = read
        .open_table(THREAD_SUMMARIES)
        .map_err(|_| index_error())?;
    let mut counts = BTreeMap::<String, u32>::new();
    for entry in table.iter().map_err(|_| index_error())? {
        let (_, value) = entry.map_err(|_| index_error())?;
        let Ok(record) = serde_json::from_slice::<ThreadSummaryRecord>(value.value()) else {
            continue;
        };
        if valid_record(&record)
            && settings
                .space(&record.space_id)
                .is_some_and(|space| !space.archived)
            && !record.archived
            && attention_status(&record.status)
        {
            let count = counts.entry(record.space_id).or_default();
            *count = count.saturating_add(1);
        }
    }
    Ok(counts)
}

pub(crate) fn last_activity(
    settings: &DesktopSettings,
) -> Result<BTreeMap<String, String>, CommandErrorDto> {
    let database = open_database()?;
    let read = database.begin_read().map_err(|_| index_error())?;
    let table = read
        .open_table(THREAD_SUMMARIES)
        .map_err(|_| index_error())?;
    let mut activity = BTreeMap::<String, String>::new();
    for entry in table.iter().map_err(|_| index_error())? {
        let (_, value) = entry.map_err(|_| index_error())?;
        let Ok(record) = serde_json::from_slice::<ThreadSummaryRecord>(value.value()) else {
            continue;
        };
        if !valid_record(&record) || settings.space(&record.space_id).is_none() {
            continue;
        }
        let latest = activity.entry(record.space_id).or_default();
        if record.updated_at > *latest {
            *latest = record.updated_at;
        }
    }
    Ok(activity)
}

fn open_database() -> Result<Database, CommandErrorDto> {
    let store = SettingsStore::open_application()?;
    let file = store.open_thread_search_file()?;
    let database = Database::builder()
        .create_file(file)
        .map_err(|_| index_error())?;
    let write = database.begin_write().map_err(|_| index_error())?;
    write
        .open_table(THREAD_SUMMARIES)
        .map_err(|_| index_error())?;
    write.commit().map_err(|_| index_error())?;
    Ok(database)
}

fn record_from_run(space_id: &str, run: &RunDto) -> Option<ThreadSummaryRecord> {
    if space_id.len() > 64
        || run.run_id.len() > 128
        || run.session_id.len() > 128
        || run.updated_at.len() > MAX_TIMESTAMP_BYTES
    {
        return None;
    }
    Some(ThreadSummaryRecord {
        schema_version: 1,
        space_id: space_id.to_owned(),
        run_id: run.run_id.clone(),
        session_id: run.session_id.clone(),
        title: bounded_text(&run.title, MAX_TITLE_BYTES),
        mode: match run.mode {
            RunModeDto::Execute => "execute",
            RunModeDto::Plan => "plan",
            RunModeDto::Research => "research",
        }
        .into(),
        status: match run.status {
            RunStatusDto::Queued => "queued",
            RunStatusDto::Running => "running",
            RunStatusDto::Waiting => "waiting",
            RunStatusDto::Cancelling => "cancelling",
            RunStatusDto::Completed => "completed",
            RunStatusDto::Failed => "failed",
            RunStatusDto::Cancelled => "cancelled",
            RunStatusDto::Interrupted => "interrupted",
            RunStatusDto::OutcomeUnknown => "outcome_unknown",
        }
        .into(),
        updated_at: run.updated_at.clone(),
        archived: run.archived,
    })
}

fn valid_record(record: &ThreadSummaryRecord) -> bool {
    record.schema_version == 1
        && !record.space_id.is_empty()
        && record.space_id.len() <= 64
        && !record.run_id.is_empty()
        && record.run_id.len() <= 128
        && !record.session_id.is_empty()
        && record.session_id.len() <= 128
        && record.title.len() <= MAX_TITLE_BYTES
        && record.updated_at.len() <= MAX_TIMESTAMP_BYTES
        && matches!(record.mode.as_str(), "execute" | "plan" | "research")
        && matches!(
            record.status.as_str(),
            "queued"
                | "running"
                | "waiting"
                | "cancelling"
                | "completed"
                | "failed"
                | "cancelled"
                | "interrupted"
                | "outcome_unknown"
        )
}

fn attention_status(status: &str) -> bool {
    matches!(status, "waiting" | "failed" | "outcome_unknown")
}

fn bounded_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn index_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "desktop_search_index",
        "The Desktop search index is unavailable. Open the Space to rebuild it.",
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_never_includes_successful_or_cancelled_runs() {
        assert!(attention_status("waiting"));
        assert!(attention_status("failed"));
        assert!(attention_status("outcome_unknown"));
        assert!(!attention_status("completed"));
        assert!(!attention_status("cancelled"));
    }

    #[test]
    fn bounded_text_preserves_utf8_boundaries() {
        assert_eq!(bounded_text("abéz", 3), "ab");
    }

    #[test]
    fn thread_search_results_keep_only_the_newest_matching_run() {
        let result = |run_id: &str, session_id: &str, updated_at: &str| SpaceSearchResultDto {
            space_id: "space-a".into(),
            space_name: "Space A".into(),
            target_id: "space-a".into(),
            run_id: run_id.into(),
            session_id: session_id.into(),
            title: run_id.into(),
            mode: "execute".into(),
            status: "completed".into(),
            updated_at: updated_at.into(),
            archived: false,
            thread_archived: false,
            attention: false,
        };
        let mut by_thread = BTreeMap::new();
        for candidate in [
            result("run-old", "session-a", "2026-08-17T10:00:00Z"),
            result("run-new", "session-a", "2026-08-17T11:00:00Z"),
            result("run-other", "session-b", "2026-08-17T09:00:00Z"),
        ] {
            insert_latest_thread_result(&mut by_thread, candidate);
        }

        assert_eq!(by_thread.len(), 2);
        assert_eq!(
            by_thread
                .get(&("space-a".into(), "session-a".into()))
                .expect("session result")
                .run_id,
            "run-new"
        );
    }
}
