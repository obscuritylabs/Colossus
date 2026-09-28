//! Bounded text comparisons. Only named files from current status/listed history
//! are admitted; renderer input never supplies blob IDs or revision expressions.
use std::path::{Path, PathBuf};

use git2::{Oid, Patch};

use super::{
    discovery::RepositoryBinding,
    dto::{DiffHunk, DiffLine, DiffRequest, DiffSource, FileDiff, FileVersion},
    error,
    reader::{Reader, index_stamp},
};
use crate::{dto::CommandErrorDto, workspace_files};

const MAX_LINES: usize = 6_000;
const MAX_DIFF_LINES: usize = 2_000;
const MAX_HUNKS: usize = 128;

/// Also applies to historical/deleted paths, for which filesystem resolution is
/// impossible. Historical content retains the file viewer's protected-name rules.
fn visible_path(path: &str) -> Result<(), CommandErrorDto> {
    if path.is_empty()
        || path.len() > 2048
        || path.contains(['\\', ':'])
        || path.split('/').any(|part| {
            matches!(part, "" | "." | "..")
                || workspace_files::hidden_entry(part)
                || !workspace_files::renderer_safe_name(part)
        })
    {
        return Err(error("This path is not available in the file viewer."));
    }
    Ok(())
}

fn repository_path(binding: &RepositoryBinding, path: &str) -> Result<PathBuf, CommandErrorDto> {
    visible_path(path)?;
    Ok(binding
        .workspace
        .strip_prefix(&binding.root.path)
        .map_err(|_| error("The repository scope changed. Refresh Git."))?
        .join(path))
}

fn version(state: &'static str, size_bytes: Option<usize>) -> FileVersion {
    FileVersion {
        state,
        content: None,
        size_bytes,
    }
}

fn text_version(bytes: &[u8]) -> FileVersion {
    if bytes.len() as u64 > workspace_files::MAX_FILE_BYTES {
        return version("too_large", Some(bytes.len()));
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return version("binary", Some(bytes.len()));
    };
    if text.chars().any(workspace_files::unsafe_text_character) {
        return version("binary", Some(bytes.len()));
    }
    if text.split('\n').count() > MAX_LINES {
        return version("too_large", Some(bytes.len()));
    }
    FileVersion {
        state: "text",
        content: Some(text.to_owned()),
        size_bytes: Some(bytes.len()),
    }
}

type ComparedVersions = (
    FileVersion,
    FileVersion,
    Option<String>,
    Option<&'static str>,
);

impl Reader {
    pub(super) fn diff(
        &self,
        binding: &RepositoryBinding,
        request: &DiffRequest,
    ) -> Result<FileDiff, CommandErrorDto> {
        let path = repository_path(binding, &request.path)?;
        let head = self.head()?.1;
        let stamp = index_stamp(binding)?;
        let (before, after, previous_path, note) = if request.source == DiffSource::Commit {
            self.commit_versions(binding, request, &path)?
        } else {
            if request.commit_id.is_some() {
                return Err(error("A working-tree comparison cannot select a commit."));
            }
            let status = self.status(binding, String::new())?;
            let selected = status
                .files
                .iter()
                .find(|file| file.path == request.path)
                .ok_or_else(|| error("This file is no longer changed. Refresh Git."))?;
            if selected.conflicted {
                return Ok(conflict_diff(&request.path));
            }
            let index = self
                .repo
                .index()
                .map_err(|_| error("The index is unavailable."))?;
            match request.source {
                DiffSource::Staged => {
                    if selected.staged.is_none() {
                        return Err(error("This file has no staged changes. Refresh Git."));
                    }
                    let old_path = repository_path(
                        binding,
                        selected.previous_path.as_deref().unwrap_or(&request.path),
                    )?;
                    let tree = head
                        .as_deref()
                        .map(|id| {
                            Oid::from_str(id)
                                .and_then(|id| self.repo.find_commit(id))
                                .and_then(|commit| commit.tree())
                        })
                        .transpose()
                        .map_err(|_| error("HEAD is unavailable."))?;
                    (
                        self.tree_version(tree.as_ref(), &old_path)?,
                        self.index_version(&index, &path)?,
                        selected.previous_path.clone(),
                        Some("HEAD → index. Only staged changes are shown."),
                    )
                }
                DiffSource::Unstaged => {
                    if selected.unstaged.is_none() {
                        return Err(error("This file has no unstaged changes. Refresh Git."));
                    }
                    let previous = self.worktree_previous_path(binding, &path, &request.path)?;
                    let old_path =
                        repository_path(binding, previous.as_deref().unwrap_or(&request.path))?;
                    (
                        self.index_version(&index, &old_path)?,
                        worktree_version(
                            binding,
                            &request.path,
                            selected.unstaged == Some("deleted"),
                        ),
                        previous,
                        Some(
                            "Index → working file. Raw text is compared; external Git filters are not run.",
                        ),
                    )
                }
                DiffSource::Untracked => {
                    if !selected.untracked {
                        return Err(error("This file is no longer untracked. Refresh Git."));
                    }
                    (
                        version("absent", None),
                        worktree_version(binding, &request.path, false),
                        None,
                        Some("New file compared with an empty file."),
                    )
                }
                DiffSource::Commit => unreachable!(),
            }
        };
        binding.revalidate()?;
        if self.head()?.1 != head || index_stamp(binding)? != stamp {
            return Err(error("Git changed during comparison. Refresh the diff."));
        }
        let mut result = FileDiff {
            path: request.path.clone(),
            previous_path,
            language: workspace_files::language_for(&request.path).into(),
            before,
            after,
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
            truncated: false,
            note,
        };
        build_hunks(&mut result)?;
        Ok(result)
    }

    fn commit_versions(
        &self,
        binding: &RepositoryBinding,
        request: &DiffRequest,
        path: &Path,
    ) -> Result<ComparedVersions, CommandErrorDto> {
        let id = request
            .commit_id
            .as_deref()
            .ok_or_else(|| error("Choose a commit from history."))?;
        let details = self.details(binding, id)?;
        let selected = details
            .files
            .iter()
            .find(|file| file.path == request.path)
            .ok_or_else(|| error("This file is no longer in the selected commit."))?;
        let old_path = repository_path(
            binding,
            selected.previous_path.as_deref().unwrap_or(&request.path),
        )?;
        let commit = self
            .repo
            .find_commit(Oid::from_str(id).map_err(|_| error("Invalid commit."))?)
            .map_err(|_| error("This commit is unavailable."))?;
        let tree = commit
            .tree()
            .map_err(|_| error("This commit tree is unavailable."))?;
        let parent = if commit.parent_count() == 0 {
            None
        } else {
            Some(
                commit
                    .parent(0)
                    .and_then(|parent| parent.tree())
                    .map_err(|_| error("The parent commit is unavailable."))?,
            )
        };
        Ok((
            self.tree_version(parent.as_ref(), &old_path)?,
            self.tree_version(Some(&tree), path)?,
            selected.previous_path.clone(),
            Some(if parent.is_none() {
                "Initial commit compared with an empty tree."
            } else {
                "Compared with the first parent."
            }),
        ))
    }

    fn worktree_previous_path(
        &self,
        binding: &RepositoryBinding,
        path: &Path,
        relative: &str,
    ) -> Result<Option<String>, CommandErrorDto> {
        // Look up the index-side name for a working-tree rename. This
        // is native-derived and still checked against workspace scope.
        let mut options = git2::StatusOptions::new();
        options
            .renames_index_to_workdir(true)
            .no_refresh(true)
            .update_index(false);
        let entries = self
            .repo
            .statuses(Some(&mut options))
            .map_err(|_| error("Git status changed. Refresh Git."))?;
        let previous = entries.iter().find_map(|entry| {
            let delta = entry.index_to_workdir()?;
            (delta.new_file().path() == Some(path))
                .then(|| delta.old_file().path().map(Path::to_path_buf))
                .flatten()
        });
        let previous = previous
            .as_deref()
            .and_then(|p| binding.relative(p))
            .filter(|p| p != relative);

        Ok(previous)
    }

    fn tree_version(
        &self,
        tree: Option<&git2::Tree<'_>>,
        path: &Path,
    ) -> Result<FileVersion, CommandErrorDto> {
        let Some(tree) = tree else {
            return Ok(version("absent", None));
        };
        match tree.get_path(path) {
            Ok(entry) => self.blob_version(entry.id(), entry.filemode().cast_unsigned()),
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(version("absent", None)),
            Err(_) => Err(error("This file revision is unavailable.")),
        }
    }

    fn index_version(
        &self,
        index: &git2::Index,
        path: &Path,
    ) -> Result<FileVersion, CommandErrorDto> {
        match index.get_path(path, 0) {
            Some(entry) => self.blob_version(entry.id, entry.mode),
            None => Ok(version("absent", None)),
        }
    }

    fn blob_version(&self, id: Oid, mode: u32) -> Result<FileVersion, CommandErrorDto> {
        if !matches!(mode, 0o100_644 | 0o100_755) {
            return Ok(version("unsupported", None));
        }
        let (size, kind) = self
            .repo
            .odb()
            .and_then(|odb| odb.read_header(id))
            .map_err(|_| error("This file revision is unavailable."))?;
        if kind != git2::ObjectType::Blob {
            return Ok(version("unsupported", Some(size)));
        }
        if size as u64 > workspace_files::MAX_FILE_BYTES {
            return Ok(version("too_large", Some(size)));
        }
        let blob = self
            .repo
            .find_blob(id)
            .map_err(|_| error("This file revision is unavailable."))?;
        Ok(text_version(blob.content()))
    }
}

fn worktree_version(binding: &RepositoryBinding, path: &str, deleted: bool) -> FileVersion {
    if deleted {
        return version("absent", None);
    }
    match workspace_files::read_file(&binding.workspace, path) {
        Ok(file) => text_version(file.content.as_bytes()),
        Err(e) => version(
            match e.code.as_str() {
                "file_preview_too_large" => "too_large",
                "file_preview_unavailable" => "binary",
                _ => "unavailable",
            },
            None,
        ),
    }
}

fn build_hunks(result: &mut FileDiff) -> Result<(), CommandErrorDto> {
    if [&result.before, &result.after]
        .iter()
        .any(|side| !matches!(side.state, "text" | "absent"))
    {
        return Ok(());
    }
    let mut options = git2::DiffOptions::new();
    options.context_lines(3).force_text(true).minimal(true);
    let patch = Patch::from_buffers(
        result.before.content.as_deref().unwrap_or("").as_bytes(),
        None,
        result.after.content.as_deref().unwrap_or("").as_bytes(),
        None,
        Some(&mut options),
    )
    .map_err(|_| error("This comparison could not be prepared."))?;
    let (_, additions, deletions) = patch
        .line_stats()
        .map_err(|_| error("This comparison is unavailable."))?;
    result.additions = additions;
    result.deletions = deletions;
    let mut count = 0;
    for i in 0..patch.num_hunks().min(MAX_HUNKS) {
        let (hunk, length) = patch
            .hunk(i)
            .map_err(|_| error("This comparison is unavailable."))?;
        let mut lines = Vec::new();
        for j in 0..length {
            if count >= MAX_DIFF_LINES {
                result.truncated = true;
                break;
            }
            let line = patch
                .line_in_hunk(i, j)
                .map_err(|_| error("This comparison is unavailable."))?;
            let kind = match line.origin() {
                '+' => "added",
                '-' => "removed",
                ' ' => "context",
                _ => continue,
            };
            lines.push(DiffLine {
                kind,
                old_line: line.old_lineno(),
                new_line: line.new_lineno(),
            });
            count += 1;
        }
        result.hunks.push(DiffHunk {
            old_start: hunk.old_start(),
            old_lines: hunk.old_lines(),
            new_start: hunk.new_start(),
            new_lines: hunk.new_lines(),
            lines,
        });
        if result.truncated {
            break;
        }
    }
    result.truncated |= patch.num_hunks() > MAX_HUNKS;
    Ok(())
}

#[cfg(test)]
mod tests;

fn conflict_diff(path: &str) -> FileDiff {
    FileDiff {
        path: path.into(),
        previous_path: None,
        language: workspace_files::language_for(path).into(),
        before: version("conflict", None),
        after: version("conflict", None),
        hunks: Vec::new(),
        additions: 0,
        deletions: 0,
        truncated: false,
        note: Some(
            "This file has unresolved conflicts. Open the current file to inspect conflict markers.",
        ),
    }
}
