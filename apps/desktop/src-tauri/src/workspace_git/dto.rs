use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiffSource {
    Staged,
    Unstaged,
    Untracked,
    Commit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DiffRequest {
    pub workspace_id: String,
    pub repository_id: String,
    pub path: String,
    pub source: DiffSource,
    pub commit_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileVersion {
    pub state: &'static str,
    pub content: Option<String>,
    pub size_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileDiff {
    pub path: String,
    pub previous_path: Option<String>,
    pub language: String,
    pub before: FileVersion,
    pub after: FileVersion,
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
    pub truncated: bool,
    pub note: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiffLine {
    pub kind: &'static str,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StatusRequest {
    pub workspace_id: String,
    #[serde(default)]
    pub approve_repository: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HistoryRequest {
    pub workspace_id: String,
    pub repository_id: String,
    pub cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(clippy::struct_field_names)] // These are distinct opaque IPC identities.
pub(crate) struct CommitRequest {
    pub workspace_id: String,
    pub repository_id: String,
    pub commit_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitStatus {
    pub state: &'static str,
    pub repository: Option<RepositorySummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepositorySummary {
    pub id: String,
    pub name: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub linked_worktree: bool,
    pub scoped: bool,
    pub operation: Option<&'static str>,
    pub files: Vec<ChangedFile>,
    pub truncated: bool,
    pub notes: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangedFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub staged: Option<&'static str>,
    pub unstaged: Option<&'static str>,
    pub untracked: bool,
    pub conflicted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitSummary {
    pub id: String,
    pub subject: String,
    pub author: String,
    pub timestamp: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitPage {
    pub head: Option<String>,
    pub commits: Vec<CommitSummary>,
    pub next_cursor: Option<String>,
    pub limited: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitDetails {
    pub commit: CommitSummary,
    pub message: String,
    pub parents: Vec<String>,
    pub files: Vec<CommitFile>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: &'static str,
}
