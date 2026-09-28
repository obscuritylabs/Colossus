use std::{
    fs,
    io::Write as _,
    path::Path,
    time::{Duration, Instant},
};

use super::{
    discovery::{RepositoryBinding, inspect_metadata},
    dto::{ChangedFile, CommitDetails, CommitFile, CommitSummary, RepositorySummary},
    error,
};
use crate::dto::CommandErrorDto;
use git2::{Repository, RepositoryOpenFlags, Status, StatusOptions};

pub(super) const PAGE_SIZE: usize = 40;
pub(super) const HISTORY_LIMIT: usize = 400;
const FILE_LIMIT: usize = 1000;

pub(super) struct Reader {
    repo: Repository,
    _config: tempfile::NamedTempFile,
    notes: Vec<&'static str>,
}

impl Reader {
    pub fn open(binding: &RepositoryBinding) -> Result<Self, CommandErrorDto> {
        binding.revalidate()?;
        inspect_metadata(binding)?;
        let repo = Repository::open_ext(
            &binding.root.path,
            RepositoryOpenFlags::NO_SEARCH,
            std::iter::empty::<&Path>(),
        )
        .map_err(failed)?;
        if repo.is_bare()
            || fs::canonicalize(repo.path())
                .map_err(|_| error("The Git directory is unavailable."))?
                != binding.git.path
            || fs::canonicalize(repo.commondir())
                .map_err(|_| error("The shared Git directory is unavailable."))?
                != binding.common.path
        {
            return Err(error("The repository layout changed. Refresh Git."));
        }
        // No transport, custom filter, hook or executable API is used. Preserve only
        // scalar worktree comparison settings; never use external attribute/ignore paths.
        let original = repo.config().map_err(failed)?;
        let mut notes = Vec::new();
        if original
            .entries(Some("^filter\\."))
            .map_err(failed)?
            .next()
            .is_some()
        {
            notes.push("External Git filters are not run; filtered files may differ from command-line status.");
        }
        if original.get_string("core.excludesfile").is_ok() {
            notes.push(
                "Only repository ignore rules are used; your global ignore file is not read.",
            );
        }
        let filemode = original.get_bool("core.filemode").unwrap_or(!cfg!(windows));
        let ignorecase = original
            .get_bool("core.ignorecase")
            .unwrap_or(cfg!(windows));
        let autocrlf = original
            .get_string("core.autocrlf")
            .unwrap_or_else(|_| "false".into());
        let autocrlf = match autocrlf.as_str() {
            "true" => "true",
            "input" => "input",
            _ => "false",
        };
        let mut config = tempfile::NamedTempFile::new()
            .map_err(|_| error("Git inspection could not create its private configuration."))?;
        writeln!(config, "[core]\n bare = false\n filemode = {filemode}\n ignorecase = {ignorecase}\n autocrlf = {autocrlf}\n attributesfile =\n excludesfile =\n[diff]\n renames = true\n renamelimit = 200").map_err(|_| error("Git inspection configuration is unavailable."))?;
        repo.set_config(&git2::Config::open(config.path()).map_err(failed)?)
            .map_err(failed)?;
        repo.set_workdir(&binding.root.path, false)
            .map_err(failed)?;
        Ok(Self {
            repo,
            _config: config,
            notes,
        })
    }

    pub fn head(&self) -> Result<(Option<String>, Option<String>), CommandErrorDto> {
        let head = self.repo.find_reference("HEAD").map_err(failed)?;
        let branch = head
            .symbolic_target()
            .map_err(failed)?
            .and_then(|value| value.strip_prefix("refs/heads/"))
            .map(|value| display(value, 256));
        let oid = match head.resolve() {
            Ok(reference) => Some(
                reference
                    .target()
                    .ok_or_else(|| error("Git HEAD is not a commit."))?
                    .to_string(),
            ),
            Err(e) if branch.is_some() && e.code() == git2::ErrorCode::NotFound => None,
            Err(e) => return Err(failed(e)),
        };
        Ok((branch, oid))
    }

    pub fn status(
        &self,
        binding: &RepositoryBinding,
        id: String,
    ) -> Result<RepositorySummary, CommandErrorDto> {
        let (branch, head) = self.head()?;
        let index_before = index_stamp(binding)?;
        self.bound_worktree(binding)?;
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false)
            .exclude_submodules(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true)
            .include_unreadable(true)
            .no_refresh(true)
            .update_index(false);
        if let Ok(scope) = binding.workspace.strip_prefix(&binding.root.path)
            && !scope.as_os_str().is_empty()
        {
            options.disable_pathspec_match(true).pathspec(scope);
        }
        let entries = self.repo.statuses(Some(&mut options)).map_err(failed)?;
        let mut files = Vec::new();
        let mut truncated = false;
        for entry in &entries {
            let delta = entry.index_to_workdir().or_else(|| entry.head_to_index());
            let path =
                delta.and_then(|delta| delta.new_file().path().or_else(|| delta.old_file().path()));
            let Some(path) = path.and_then(|path| binding.relative(path)) else {
                truncated = true;
                continue;
            };
            if files.len() == FILE_LIMIT {
                truncated = true;
                break;
            }
            let flags = entry.status();
            let previous_path = entry
                .head_to_index()
                .and_then(|delta| delta.old_file().path())
                .and_then(|old| binding.relative(old))
                .filter(|old| old != &path);
            files.push(ChangedFile {
                path,
                previous_path,
                staged: index_status(flags),
                unstaged: worktree_status(flags),
                untracked: flags.contains(Status::WT_NEW),
                conflicted: flags.contains(Status::CONFLICTED),
            });
        }
        let has_submodules = self.add_gitlinks(binding, &mut files, &mut truncated)?;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        if self.head()? != (branch.clone(), head.clone()) {
            return Err(error("HEAD changed during refresh. Refresh Git again."));
        }
        if index_before != index_stamp(binding)? {
            return Err(error(
                "The Git index changed during refresh. Refresh Git again.",
            ));
        }
        let mut notes = self.notes.clone();
        if has_submodules {
            notes.push(
                "Submodule commits are shown, but changes inside submodules are not inspected.",
            );
        }
        Ok(RepositorySummary {
            id,
            name: display(&binding.root.display_name, 160),
            branch,
            head,
            linked_worktree: binding.git.path != binding.common.path,
            scoped: binding.workspace != binding.root.path,
            operation: match self.repo.state() {
                git2::RepositoryState::Clean => None,
                git2::RepositoryState::Merge => Some("Merge in progress"),
                git2::RepositoryState::Rebase
                | git2::RepositoryState::RebaseInteractive
                | git2::RepositoryState::RebaseMerge => Some("Rebase in progress"),
                git2::RepositoryState::CherryPick | git2::RepositoryState::CherryPickSequence => {
                    Some("Cherry-pick in progress")
                }
                git2::RepositoryState::Revert | git2::RepositoryState::RevertSequence => {
                    Some("Revert in progress")
                }
                _ => Some("Git operation in progress"),
            },
            files,
            truncated,
            notes,
        })
    }

    fn add_gitlinks(
        &self,
        binding: &RepositoryBinding,
        files: &mut Vec<ChangedFile>,
        truncated: &mut bool,
    ) -> Result<bool, CommandErrorDto> {
        let index = self.repo.index().map_err(failed)?;
        let has_submodules = index.iter().any(|entry| entry.mode == 0o160_000);
        let head = self
            .repo
            .head()
            .ok()
            .and_then(|reference| reference.peel_to_tree().ok());
        let mut options = git2::DiffOptions::new();
        options.skip_binary_check(true);
        let diff = self
            .repo
            .diff_tree_to_index(head.as_ref(), Some(&index), Some(&mut options))
            .map_err(failed)?;
        for delta in diff.deltas().filter(|delta| {
            delta.old_file().mode() == git2::FileMode::Commit
                || delta.new_file().mode() == git2::FileMode::Commit
        }) {
            let Some(path) = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .and_then(|path| binding.relative(path))
            else {
                continue;
            };
            if files.iter().any(|file| file.path == path) {
                continue;
            }
            if files.len() == FILE_LIMIT {
                *truncated = true;
                break;
            }
            files.push(ChangedFile {
                path,
                previous_path: None,
                staged: Some(match delta.status() {
                    git2::Delta::Added => "added",
                    git2::Delta::Deleted => "deleted",
                    _ => "modified",
                }),
                unstaged: None,
                untracked: false,
                conflicted: false,
            });
        }
        Ok(has_submodules)
    }

    fn bound_worktree(&self, binding: &RepositoryBinding) -> Result<(), CommandErrorDto> {
        let start = Instant::now();
        let mut pending = vec![binding.workspace.clone()];
        let mut entries = 0;
        let index = self.repo.index().map_err(failed)?;
        while let Some(directory) = pending.pop() {
            for entry in
                fs::read_dir(directory).map_err(|_| error("The working tree could not be read."))?
            {
                entries += 1;
                if entries > 100_000 || start.elapsed() > Duration::from_secs(5) {
                    return Err(error("This working tree exceeds the Git inspection limit."));
                }
                let entry = entry.map_err(|_| error("The working tree could not be read."))?;
                if entry.file_name() == ".git" {
                    continue;
                }
                let path = entry.path();
                if index
                    .get_path(
                        path.strip_prefix(&binding.root.path)
                            .map_err(|_| error("The working tree changed."))?,
                        0,
                    )
                    .is_some_and(|entry| entry.mode == 0o160_000)
                {
                    continue;
                }
                let meta = fs::symlink_metadata(&path)
                    .map_err(|_| error("The working tree changed during inspection."))?;
                if meta.is_file()
                    && meta.len() > 64 * 1024 * 1024
                    && index
                        .get_path(
                            path.strip_prefix(&binding.root.path)
                                .map_err(|_| error("The working tree changed."))?,
                            0,
                        )
                        .is_some()
                {
                    return Err(error(
                        "A tracked file exceeds the 64 MiB Git inspection limit.",
                    ));
                }
                if meta.is_dir()
                    && !meta.file_type().is_symlink()
                    && !self
                        .repo
                        .status_should_ignore(
                            path.strip_prefix(&binding.root.path)
                                .map_err(|_| error("The working tree changed."))?,
                        )
                        .map_err(failed)?
                {
                    super::discovery::reject_link(&path)?;
                    pending.push(path);
                }
            }
        }
        Ok(())
    }

    pub fn history(
        &self,
        head: &str,
        offset: usize,
    ) -> Result<(Vec<CommitSummary>, bool), CommandErrorDto> {
        if offset >= HISTORY_LIMIT {
            return Err(error(
                "The history page is no longer available. Refresh Git.",
            ));
        }
        let mut walk = self.repo.revwalk().map_err(failed)?;
        // Time order permits incremental walking; a topological sort can visit
        // the entire graph before yielding the first bounded page.
        walk.set_sorting(git2::Sort::TIME).map_err(failed)?;
        walk.push(git2::Oid::from_str(head).map_err(failed)?)
            .map_err(failed)?;
        let mut values = Vec::new();
        for oid in walk.skip(offset).take(PAGE_SIZE + 1) {
            let commit = self
                .repo
                .find_commit(oid.map_err(failed)?)
                .map_err(failed)?;
            values.push(summary(&commit));
        }
        let more = values.len() > PAGE_SIZE;
        values.truncate(PAGE_SIZE);
        Ok((values, more))
    }

    pub fn details(
        &self,
        binding: &RepositoryBinding,
        id: &str,
    ) -> Result<CommitDetails, CommandErrorDto> {
        let commit = self
            .repo
            .find_commit(git2::Oid::from_str(id).map_err(failed)?)
            .map_err(failed)?;
        let tree = commit.tree().map_err(failed)?;
        let parent = if commit.parent_count() == 0 {
            None
        } else {
            Some(commit.parent(0).map_err(failed)?.tree().map_err(failed)?)
        };
        let mut options = git2::DiffOptions::new();
        options
            .skip_binary_check(true)
            .include_typechange(true)
            .ignore_submodules(true);
        if let Ok(scope) = binding.workspace.strip_prefix(&binding.root.path)
            && !scope.as_os_str().is_empty()
        {
            options.disable_pathspec_match(true).pathspec(scope);
        }
        let mut diff = self
            .repo
            .diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut options))
            .map_err(failed)?;
        diff.find_similar(Some(
            git2::DiffFindOptions::new().renames(true).rename_limit(200),
        ))
        .map_err(failed)?;
        let mut files = Vec::new();
        let mut truncated = false;
        for delta in diff.deltas() {
            let Some(path) = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .and_then(|path| binding.relative(path))
            else {
                continue;
            };
            if files.len() == FILE_LIMIT {
                truncated = true;
                break;
            }
            let previous_path = delta
                .old_file()
                .path()
                .and_then(|old| binding.relative(old))
                .filter(|old| old != &path);
            files.push(CommitFile {
                path,
                previous_path,
                status: match delta.status() {
                    git2::Delta::Added => "added",
                    git2::Delta::Deleted => "deleted",
                    git2::Delta::Renamed => "renamed",
                    git2::Delta::Typechange => "type changed",
                    _ => "modified",
                },
            });
        }
        let message = display(
            commit.message().unwrap_or("Commit message is not UTF-8"),
            16_384,
        );
        truncated |= commit.message_bytes().len() > 16_384 || commit.parent_count() > 16;
        Ok(CommitDetails {
            commit: summary(&commit),
            message,
            parents: commit
                .parent_ids()
                .take(16)
                .map(|id| id.to_string())
                .collect(),
            files,
            truncated,
        })
    }
}

fn summary(commit: &git2::Commit<'_>) -> CommitSummary {
    CommitSummary {
        id: commit.id().to_string(),
        subject: display(
            commit.summary().ok().flatten().unwrap_or("Untitled commit"),
            300,
        ),
        author: display(commit.author().name().unwrap_or("Unknown author"), 160),
        timestamp: commit.time().seconds(),
    }
}
fn display(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| {
            (!c.is_control() || matches!(c, '\n' | '\t'))
                && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(limit)
        .collect()
}
fn index_status(flags: Status) -> Option<&'static str> {
    if flags.contains(Status::INDEX_NEW) {
        Some("added")
    } else if flags.contains(Status::INDEX_DELETED) {
        Some("deleted")
    } else if flags.contains(Status::INDEX_RENAMED) {
        Some("renamed")
    } else if flags.contains(Status::INDEX_TYPECHANGE) {
        Some("type changed")
    } else if flags.contains(Status::INDEX_MODIFIED) {
        Some("modified")
    } else {
        None
    }
}
fn worktree_status(flags: Status) -> Option<&'static str> {
    if flags.contains(Status::WT_DELETED) {
        Some("deleted")
    } else if flags.contains(Status::WT_RENAMED) {
        Some("renamed")
    } else if flags.contains(Status::WT_TYPECHANGE) {
        Some("type changed")
    } else if flags.contains(Status::WT_UNREADABLE) {
        Some("unreadable")
    } else if flags.contains(Status::WT_MODIFIED) {
        Some("modified")
    } else {
        None
    }
}
fn failed(_: git2::Error) -> CommandErrorDto {
    error("Git could not read this repository. Check its metadata and refresh.")
}

fn index_stamp(
    binding: &RepositoryBinding,
) -> Result<Option<(u64, std::time::SystemTime)>, CommandErrorDto> {
    match fs::metadata(binding.git.path.join("index")) {
        Ok(metadata) => Ok(Some((
            metadata.len(),
            metadata
                .modified()
                .map_err(|_| error("The Git index is unavailable."))?,
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(error("The Git index is unavailable.")),
    }
}
