use super::*;

/// Permit-bound filesystem adapter.
#[derive(Default)]
pub struct FilesystemExecutor {
    workspace_search_exclusions: Vec<PathBuf>,
}

impl FilesystemExecutor {
    /// Construct the filesystem adapter. Authorization still requires a permit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Omit host-owned control content from workspace-scoped discovery. This
    /// does not change explicit filesystem access or widen any permit.
    pub fn with_workspace_search_exclusions(mut self, paths: Vec<PathBuf>) -> Self {
        self.workspace_search_exclusions = paths;
        self
    }
}

#[async_trait]
impl EffectExecutor for FilesystemExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let mode = filesystem_mode(&request.action)?;
        let target = authorized_path(Path::new(&request.resource), mode, permit.obligations())?;
        let max_output =
            usize::try_from(permit.obligations().max_output_bytes).map_err(adapter_failure)?;
        match request.action.as_str() {
            "filesystem.read" | "filesystem.read_run_input" => {
                let metadata = fs::metadata(&target).map_err(adapter_failure)?;
                if !metadata.is_file() {
                    return Err(adapter_failure("filesystem.read requires a regular file"));
                }
                if metadata.len() > permit.obligations().max_output_bytes {
                    return Err(adapter_failure("file exceeds the permitted output bound"));
                }
                let bytes = fs::read(target).map_err(adapter_failure)?;
                Ok(QuarantinedEffectResult {
                    media_type: "application/octet-stream".into(),
                    bytes,
                    effect_succeeded: true,
                })
            }
            "filesystem.metadata" => {
                let metadata = fs::metadata(&target).map_err(adapter_failure)?;
                bounded_json(
                    json!({
                        "is_file": metadata.is_file(),
                        "is_directory": metadata.is_dir(),
                        "length": metadata.len(),
                        "readonly": metadata.permissions().readonly(),
                    }),
                    max_output,
                )
            }
            "filesystem.list" => {
                let mut entries = fs::read_dir(&target)
                    .map_err(adapter_failure)?
                    .map(|entry| {
                        let entry = entry.map_err(adapter_failure)?;
                        let metadata =
                            fs::symlink_metadata(entry.path()).map_err(adapter_failure)?;
                        Ok(json!({
                            "name": entry.file_name().to_string_lossy(),
                            "is_file": metadata.is_file(),
                            "is_directory": metadata.is_dir(),
                            "length": metadata.len(),
                        }))
                    })
                    .collect::<Result<Vec<_>, ExecutionError>>()?;
                entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
                bounded_json(json!({"entries": entries}), max_output)
            }
            "filesystem.search" => search_files(
                &target,
                &request.content,
                max_output,
                permit.obligations().resource_authority == ResourceAuthority::Ambient,
                &self.workspace_search_exclusions,
            ),
            "filesystem.write" | "audit.export.write" => {
                write_file(&target, &request.content, max_output)
            }
            "patch.preview" => preview_patch(&target, &request.content, max_output),
            "patch.apply" | "patch.reverse" | "trace.export" => {
                write_file(&target, &request.content, max_output)
            }
            _ => Err(adapter_failure("unsupported filesystem action")),
        }
    }
}

pub(super) fn filesystem_mode(action: &str) -> Result<&'static str, ExecutionError> {
    match action {
        "filesystem.read"
        | "filesystem.read_run_input"
        | "filesystem.list"
        | "filesystem.search"
        | "patch.preview" => Ok("read"),
        "filesystem.metadata" => Ok("metadata"),
        "filesystem.write" | "patch.apply" | "patch.reverse" | "trace.export"
        | "audit.export.write" => Ok("write"),
        _ => Err(adapter_failure("unsupported filesystem action")),
    }
}

pub(super) const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
pub(super) const MAX_SEARCH_LINE_BYTES: usize = 4096;

pub(super) fn search_files(
    root: &Path,
    content: &Value,
    max_output: usize,
    ambient: bool,
    workspace_search_exclusions: &[PathBuf],
) -> Result<QuarantinedEffectResult, ExecutionError> {
    if !root.is_dir() {
        return Err(adapter_failure(
            "filesystem.search requires a directory root",
        ));
    }
    let pattern = content
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| adapter_failure("filesystem.search pattern is absent"))?;
    if pattern.is_empty() || pattern.len() > 4096 {
        return Err(adapter_failure(
            "filesystem.search pattern must contain 1..=4096 bytes",
        ));
    }
    let case_sensitive = content
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let regex_enabled = content
        .get("regex")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let max_matches = content
        .get("max_matches")
        .and_then(Value::as_u64)
        .unwrap_or(100);
    if !(1..=1000).contains(&max_matches) {
        return Err(adapter_failure(
            "filesystem.search max_matches must be in 1..=1000",
        ));
    }
    let context_lines = content
        .get("context_lines")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if context_lines > 20 {
        return Err(adapter_failure(
            "filesystem.search context_lines must be in 0..=20",
        ));
    }
    let context_lines = usize::try_from(context_lines).unwrap_or(20);
    let workspace_scoped = content
        .get("workspace_scoped")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let respect_repository_ignores = !ambient || workspace_scoped;
    let glob = content
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|value| {
            Glob::new(value)
                .map(|glob| glob.compile_matcher())
                .map_err(adapter_failure)
        })
        .transpose()?;
    let matcher = SearchMatcher::new(pattern, regex_enabled, case_sensitive)?;
    let mut matches = Vec::new();
    let mut truncated = false;
    let mut walker = WalkBuilder::new(root);
    let exclusions = if workspace_scoped {
        workspace_search_exclusions.to_vec()
    } else {
        Vec::new()
    };
    walker
        .follow_links(false)
        .filter_entry(move |entry| !exclusions.iter().any(|root| entry.path().starts_with(root)))
        .hidden(false)
        .ignore(respect_repository_ignores)
        .git_ignore(respect_repository_ignores)
        .git_global(respect_repository_ignores)
        .git_exclude(respect_repository_ignores)
        .max_filesize(Some(MAX_SEARCH_FILE_BYTES));
    for entry in walker.build().filter_map(Result::ok) {
        let path = entry.path();
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let relative = path.strip_prefix(root).map_err(adapter_failure)?;
        if ((!ambient || workspace_scoped) && is_control_path(relative))
            || !glob_matches(glob.as_ref(), relative)
        {
            continue;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        if bytes.len() > usize::try_from(MAX_SEARCH_FILE_BYTES).unwrap_or(usize::MAX)
            || bytes.contains(&0)
        {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let lines = text.lines().collect::<Vec<_>>();
        for (line_index, line) in lines.iter().copied().enumerate() {
            let Some(column) = matcher.find(line) else {
                continue;
            };
            let text = search_match_text(&lines, line_index, context_lines);
            matches.push(json!({
                "path": relative.to_string_lossy(),
                "line": line_index.saturating_add(1),
                "column": column.saturating_add(1),
                "text": text,
            }));
            if matches.len() >= usize::try_from(max_matches).unwrap_or(usize::MAX) {
                truncated = true;
                break;
            }
            if serde_json::to_vec(&json!({"matches": matches, "truncated": false}))
                .is_ok_and(|bytes| bytes.len() > max_output)
            {
                matches.pop();
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }
    bounded_json(
        json!({"matches": matches, "truncated": truncated}),
        max_output,
    )
}

fn search_match_text(lines: &[&str], line_index: usize, context_lines: usize) -> String {
    if context_lines == 0 {
        return bounded_search_line(lines[line_index]).to_owned();
    }
    let start = line_index.saturating_sub(context_lines);
    let end = line_index
        .saturating_add(context_lines)
        .saturating_add(1)
        .min(lines.len());
    lines[start..end]
        .iter()
        .enumerate()
        .map(|(offset, line)| format!("{}: {}", start + offset + 1, bounded_search_line(line)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) enum SearchMatcher {
    Regex(Regex),
    Literal {
        pattern: String,
        case_sensitive: bool,
    },
}

impl SearchMatcher {
    fn new(
        pattern: &str,
        regex_enabled: bool,
        case_sensitive: bool,
    ) -> Result<Self, ExecutionError> {
        if regex_enabled {
            RegexBuilder::new(pattern)
                .case_insensitive(!case_sensitive)
                .size_limit(1024 * 1024)
                .build()
                .map(Self::Regex)
                .map_err(adapter_failure)
        } else {
            Ok(Self::Literal {
                pattern: if case_sensitive {
                    pattern.into()
                } else {
                    pattern.to_lowercase()
                },
                case_sensitive,
            })
        }
    }

    fn find(&self, line: &str) -> Option<usize> {
        match self {
            Self::Regex(regex) => regex.find(line).map(|found| found.start()),
            Self::Literal {
                pattern,
                case_sensitive,
            } if *case_sensitive => line.find(pattern),
            Self::Literal { pattern, .. } => line.to_lowercase().find(pattern),
        }
    }
}

pub(super) fn glob_matches(matcher: Option<&GlobMatcher>, relative: &Path) -> bool {
    matcher.is_none_or(|matcher| matcher.is_match(relative))
}

pub(super) fn is_control_path(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str();
        value == ".colossus" || value == ".git"
    })
}

pub(super) fn bounded_search_line(line: &str) -> &str {
    if line.len() <= MAX_SEARCH_LINE_BYTES {
        return line;
    }
    let mut end = MAX_SEARCH_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &line[..end]
}

pub(super) fn authorized_path(
    requested: &Path,
    mode: &str,
    obligations: &PolicyObligations,
) -> Result<PathBuf, ExecutionError> {
    if !requested.is_absolute() {
        return Err(adapter_failure("effect paths must be absolute"));
    }
    if fs::symlink_metadata(requested).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(adapter_failure("symbolic-link effect targets are rejected"));
    }
    let target = if mode == "write" && !requested.exists() {
        let parent = requested
            .parent()
            .ok_or_else(|| adapter_failure("write target has no parent"))?;
        let filename = requested
            .file_name()
            .ok_or_else(|| adapter_failure("write target has no filename"))?;
        fs::canonicalize(parent)
            .map(|parent| parent.join(filename))
            .map_err(adapter_failure)?
    } else {
        fs::canonicalize(requested).map_err(adapter_failure)?
    };
    if obligations.resource_authority == ResourceAuthority::Ambient {
        return Ok(target);
    }
    let allowed = obligations.filesystem.iter().any(|grant| {
        let mode_allowed = grant.mode == "write"
            || grant.mode == mode
            || (mode == "metadata" && grant.mode == "read");
        mode_allowed && fs::canonicalize(&grant.root).is_ok_and(|root| target.starts_with(root))
    });
    if !allowed {
        return Err(adapter_failure(format!(
            "{} is outside permitted {mode} roots",
            requested.display()
        )));
    }
    Ok(target)
}

pub(super) fn proposed_write_bytes(
    content: &Value,
    limit: usize,
) -> Result<Vec<u8>, ExecutionError> {
    let bytes = if let Some(encoded) = content.get("content_base64").and_then(Value::as_str) {
        BASE64.decode(encoded).map_err(adapter_failure)?
    } else if let Some(text) = content.get("text").and_then(Value::as_str) {
        text.as_bytes().to_vec()
    } else {
        return Err(adapter_failure(
            "filesystem.write requires text or content_base64",
        ));
    };
    if bytes.len() > limit {
        return Err(adapter_failure("write content exceeds the permitted bound"));
    }
    Ok(bytes)
}

pub(super) fn write_file(
    target: &Path,
    content: &Value,
    max_output: usize,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    if content.get("content_base64").is_some() {
        let bytes = proposed_write_bytes(content, max_output)?;
        let result = json!({
            "bytes_written": bytes.len(),
            "sha256": sha256_hex(&bytes),
        });
        let encoded = bounded_json_bytes(&result, max_output)?;
        atomic_write(target, &bytes)?;
        return Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: encoded,
            effect_succeeded: true,
        });
    }

    let operation = content
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("write");
    let existing = existing_text(target, max_output)?;
    let (updated, replacements, create_only) = match operation {
        "write" => {
            let supplied = content
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| adapter_failure("filesystem.write text is absent"))?;
            let mode = content
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("overwrite");
            match mode {
                "create" if existing.is_some() => {
                    return Err(adapter_failure(
                        "filesystem.write create mode refuses an existing file",
                    ));
                }
                "create" => (supplied.to_owned(), None, true),
                "overwrite" => (supplied.to_owned(), None, false),
                "append" => (
                    format!("{}{}", existing.as_deref().unwrap_or_default(), supplied),
                    None,
                    false,
                ),
                _ => {
                    return Err(adapter_failure(
                        "filesystem.write mode must be create, overwrite, or append",
                    ));
                }
            }
        }
        "replace" => {
            let original = existing
                .as_deref()
                .ok_or_else(|| adapter_failure("filesystem.replace requires an existing file"))?;
            let old = content
                .get("old")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| adapter_failure("filesystem.replace old text is absent"))?;
            let new = content
                .get("new")
                .and_then(Value::as_str)
                .ok_or_else(|| adapter_failure("filesystem.replace new text is absent"))?;
            let replace_all = content
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let occurrences = original.matches(old).count();
            if occurrences == 0 {
                return Err(adapter_failure("filesystem.replace old text was not found"));
            }
            if occurrences > 1 && !replace_all {
                return Err(adapter_failure("filesystem.replace old text is ambiguous"));
            }
            (
                if replace_all {
                    original.replace(old, new)
                } else {
                    original.replacen(old, new, 1)
                },
                Some(if replace_all { occurrences } else { 1 }),
                false,
            )
        }
        _ => return Err(adapter_failure("unknown filesystem mutation operation")),
    };
    if updated.len() > max_output {
        return Err(adapter_failure(
            "filesystem mutation content exceeds the permitted bound",
        ));
    }
    let original = existing.as_deref().unwrap_or_default();
    let display_path = mutation_display_path(content, target);
    let mut result = json!({
        "path": display_path,
        "bytes_written": updated.len(),
        "sha256": sha256_hex(updated.as_bytes()),
        "diff": compact_unified_diff(display_path, original, &updated, max_output / 2),
        "changed_line_ranges": changed_line_ranges(original, &updated),
    });
    if let Some(replacements) = replacements {
        result["replacements"] = json!(replacements);
    }
    let encoded = bounded_json_bytes(&result, max_output)?;
    if create_only {
        atomic_create(target, updated.as_bytes())?;
    } else {
        atomic_write(target, updated.as_bytes())?;
    }
    Ok(QuarantinedEffectResult {
        media_type: "application/json".into(),
        bytes: encoded,
        effect_succeeded: true,
    })
}

pub(super) fn preview_patch(
    target: &Path,
    content: &Value,
    max_output: usize,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let original = existing_text(target, max_output)?
        .ok_or_else(|| adapter_failure("patch.preview requires an existing file"))?;
    let old = content
        .get("old")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| adapter_failure("patch.preview old text is absent"))?;
    let new = content
        .get("new")
        .and_then(Value::as_str)
        .ok_or_else(|| adapter_failure("patch.preview new text is absent"))?;
    let replace_all = content
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let occurrences = original.matches(old).count();
    if occurrences == 0 {
        return Err(adapter_failure("patch.preview old text was not found"));
    }
    if occurrences > 1 && !replace_all {
        return Err(adapter_failure("patch.preview old text is ambiguous"));
    }
    let updated = if replace_all {
        original.replace(old, new)
    } else {
        original.replacen(old, new, 1)
    };
    if updated.len() > max_output {
        return Err(adapter_failure("patch preview exceeds the permitted bound"));
    }
    let display_path = mutation_display_path(content, target);
    bounded_json(
        json!({
            "path": display_path,
            "replacements": if replace_all { occurrences } else { 1 },
            "diff": compact_unified_diff(display_path, &original, &updated, max_output / 2),
            "changed_line_ranges": changed_line_ranges(&original, &updated),
        }),
        max_output,
    )
}

pub(super) fn existing_text(
    target: &Path,
    max_bytes: usize,
) -> Result<Option<String>, ExecutionError> {
    match fs::metadata(target) {
        Ok(metadata) if !metadata.is_file() => Err(adapter_failure(
            "filesystem mutation requires a regular file target",
        )),
        Ok(metadata) if metadata.len() > u64::try_from(max_bytes).unwrap_or(u64::MAX) => Err(
            adapter_failure("existing file exceeds the permitted mutation bound"),
        ),
        Ok(_) => fs::read_to_string(target)
            .map(Some)
            .map_err(adapter_failure),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(adapter_failure(error)),
    }
}

pub(super) fn mutation_display_path<'a>(content: &'a Value, target: &'a Path) -> &'a str {
    content
        .get("display_path")
        .and_then(Value::as_str)
        .filter(|value| !value.contains('\n') && !value.contains('\r'))
        .or_else(|| target.file_name().and_then(|name| name.to_str()))
        .unwrap_or("file")
}

pub(super) fn compact_unified_diff(path: &str, old: &str, new: &str, max_bytes: usize) -> String {
    if old == new {
        return String::new();
    }
    let old_lines = old.lines().collect::<Vec<_>>();
    let new_lines = new.lines().collect::<Vec<_>>();
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let context_start = prefix.saturating_sub(3);
    let old_end = old_lines.len().saturating_sub(suffix);
    let new_end = new_lines.len().saturating_sub(suffix);
    let context_end_old = old_end.saturating_add(3).min(old_lines.len());
    let context_end_new = new_end.saturating_add(3).min(new_lines.len());
    let mut diff = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        context_start.saturating_add(1),
        context_end_old.saturating_sub(context_start),
        context_start.saturating_add(1),
        context_end_new.saturating_sub(context_start),
    );
    for line in &old_lines[context_start..prefix] {
        diff.push(' ');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &old_lines[prefix..old_end] {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &new_lines[prefix..new_end] {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &new_lines[new_end..context_end_new] {
        diff.push(' ');
        diff.push_str(line);
        diff.push('\n');
    }
    truncate_diff(diff, max_bytes.max(256))
}

pub(super) fn truncate_diff(mut diff: String, max_bytes: usize) -> String {
    if diff.len() <= max_bytes {
        return diff;
    }
    let marker = "\n... diff truncated ...\n";
    let mut end = max_bytes.saturating_sub(marker.len()).min(diff.len());
    while !diff.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    diff.truncate(end);
    diff.push_str(marker);
    diff
}

pub(super) fn changed_line_ranges(old: &str, new: &str) -> Vec<Value> {
    if old == new {
        return Vec::new();
    }
    let old_lines = old.lines().collect::<Vec<_>>();
    let new_lines = new.lines().collect::<Vec<_>>();
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let new_end = new_lines.len().saturating_sub(suffix);
    let start = prefix.saturating_add(1);
    let end = if new_end == prefix {
        start.min(new_lines.len().max(1))
    } else {
        new_end
    };
    vec![json!({"start": start, "end": end})]
}

pub(super) fn bounded_json_bytes(value: &Value, limit: usize) -> Result<Vec<u8>, ExecutionError> {
    let bytes = serde_json::to_vec(value).map_err(adapter_failure)?;
    if bytes.len() > limit {
        return Err(adapter_failure(
            "adapter output exceeds the permitted bound",
        ));
    }
    Ok(bytes)
}

pub(super) fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), ExecutionError> {
    let parent = target
        .parent()
        .ok_or_else(|| adapter_failure("write target has no parent"))?;
    let temporary = parent.join(format!(".colossus-write-{}.tmp", Uuid::now_v7()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(adapter_failure)?;
        file.write_all(bytes).map_err(adapter_failure)?;
        file.sync_all().map_err(adapter_failure)?;
        fs::rename(&temporary, target).map_err(adapter_failure)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn atomic_create(target: &Path, bytes: &[u8]) -> Result<(), ExecutionError> {
    let parent = target
        .parent()
        .ok_or_else(|| adapter_failure("create target has no parent"))?;
    let temporary = parent.join(format!(".colossus-create-{}.tmp", Uuid::now_v7()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(adapter_failure)?;
        file.write_all(bytes).map_err(adapter_failure)?;
        file.sync_all().map_err(adapter_failure)?;
        drop(file);
        fs::hard_link(&temporary, target).map_err(adapter_failure)?;
        let _ = fs::remove_file(&temporary);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn bounded_json(
    value: Value,
    limit: usize,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let bytes = serde_json::to_vec(&value).map_err(adapter_failure)?;
    if bytes.len() > limit {
        return Err(adapter_failure(
            "adapter output exceeds the permitted bound",
        ));
    }
    Ok(QuarantinedEffectResult {
        media_type: "application/json".into(),
        bytes,
        effect_succeeded: true,
    })
}
