use super::*;

pub(super) const MAX_FILE_SUMMARY_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_FILE_SUMMARY_PATH_JSON_BYTES: usize = 8 * 1024;
const MAX_FILE_SUMMARY_PREVIEW_JSON_BYTES: usize = 24 * 1024;
const MAX_FILE_SUMMARY_COLLECTION_JSON_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum RepositoryOperation {
    Map {
        path: String,
        max_files: usize,
    },
    SymbolSearch {
        pattern: String,
        path: String,
        max_results: usize,
    },
    References {
        symbol: String,
        path: String,
        max_results: usize,
    },
    FileSummary {
        path: String,
        max_lines: usize,
    },
}

impl RepositoryOperation {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Map { .. } => "repo.map",
            Self::SymbolSearch { .. } => "repo.symbol_search",
            Self::References { .. } => "repo.references",
            Self::FileSummary { .. } => "repo.file_summary",
        }
    }

    pub(super) fn resource(&self) -> &str {
        match self {
            Self::Map { path, .. }
            | Self::SymbolSearch { path, .. }
            | Self::References { path, .. }
            | Self::FileSummary { path, .. } => path,
        }
    }

    /// Replaces the operation path with its validated workspace-relative spelling so the
    /// executor observes the same confined path the gateway authorized.
    pub(super) fn with_resource(mut self, resource: String) -> Self {
        match &mut self {
            Self::Map { path, .. }
            | Self::SymbolSearch { path, .. }
            | Self::References { path, .. }
            | Self::FileSummary { path, .. } => *path = resource,
        }
        self
    }
}

pub(super) struct RepositoryEffectExecutor {
    pub(super) workspace: PathBuf,
}

impl RepositoryEffectExecutor {
    pub(super) fn resolve(&self, resource: &str, ambient: bool) -> Result<PathBuf, ExecutionError> {
        let requested = Path::new(resource);
        if resource.contains('\0')
            || (!ambient
                && (requested.is_absolute()
                    || requested.components().any(|component| {
                        matches!(component, std::path::Component::ParentDir)
                            || matches!(component.as_os_str().to_str(), Some(".git" | ".colossus"))
                    })))
        {
            return Err(ExecutionError::Failed(
                "repository paths must remain inside the workspace and outside control state"
                    .into(),
            ));
        }
        let joined = if ambient && requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.workspace.join(requested)
        };
        if fs::symlink_metadata(&joined)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(ExecutionError::Failed(
                "repository operation roots cannot be symbolic links".into(),
            ));
        }
        let canonical =
            fs::canonicalize(&joined).map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if !ambient && !canonical.starts_with(&self.workspace) {
            return Err(ExecutionError::Failed(
                "repository path escaped the active workspace".into(),
            ));
        }
        Ok(canonical)
    }

    pub(super) fn files(
        &self,
        root: &Path,
        maximum: usize,
        ambient: bool,
    ) -> Result<(Vec<PathBuf>, bool), ExecutionError> {
        let mut files = Vec::new();
        let mut truncated = false;
        let hard_limit = maximum.clamp(1, 5_000);
        let mut walker = WalkBuilder::new(root);
        walker
            .follow_links(false)
            .hidden(false)
            .ignore(!ambient)
            .git_ignore(!ambient)
            .git_global(!ambient)
            .git_exclude(!ambient)
            .parents(false);
        let walker = walker.build();
        for entry in walker {
            let entry = entry.map_err(|error| ExecutionError::Failed(error.to_string()))?;
            let boundary = if ambient { root } else { &self.workspace };
            let relative = entry.path().strip_prefix(boundary).map_err(|_| {
                ExecutionError::Failed("repository walk escaped its authorized root".into())
            })?;
            if !ambient
                && relative.components().any(|component| {
                    matches!(component.as_os_str().to_str(), Some(".git" | ".colossus"))
                })
            {
                continue;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let canonical = fs::canonicalize(entry.path())
                .map_err(|error| ExecutionError::Failed(error.to_string()))?;
            if !canonical.starts_with(boundary) {
                return Err(ExecutionError::Failed(
                    "repository walk escaped its authorized root".into(),
                ));
            }
            if files.len() == hard_limit {
                truncated = true;
                break;
            }
            files.push(canonical);
        }
        files.sort();
        Ok((files, truncated))
    }

    pub(super) fn relative(&self, path: &Path, ambient: bool) -> Result<String, ExecutionError> {
        if ambient {
            return Ok(path.display().to_string());
        }
        path.strip_prefix(&self.workspace)
            .map(|path| {
                if path.as_os_str().is_empty() {
                    ".".into()
                } else {
                    path.to_string_lossy().into_owned()
                }
            })
            .map_err(|_| ExecutionError::Failed("repository result escaped workspace".into()))
    }

    pub(super) fn bounded_text(&self, path: &Path) -> Result<Option<String>, ExecutionError> {
        let metadata =
            fs::metadata(path).map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if metadata.len() > 1024 * 1024 {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if bytes.contains(&0) {
            return Ok(None);
        }
        Ok(String::from_utf8(bytes).ok())
    }

    pub(super) fn map(
        &self,
        path: &str,
        max_files: usize,
        ambient: bool,
    ) -> Result<Value, ExecutionError> {
        let root = self.resolve(path, ambient)?;
        if !root.is_dir() {
            return Err(ExecutionError::Failed(
                "repo.map path must be a directory".into(),
            ));
        }
        let (files, truncated) = self.files(&root, max_files.clamp(1, 1_000), ambient)?;
        let entries = files
            .iter()
            .map(|file| {
                let metadata = fs::metadata(file)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?;
                Ok(json!({
                    "path": self.relative(file, ambient)?,
                    "bytes": metadata.len(),
                    "extension": file.extension().and_then(|value| value.to_str()),
                }))
            })
            .collect::<Result<Vec<_>, ExecutionError>>()?;
        let mut extension_counts = BTreeMap::<String, usize>::new();
        for entry in &entries {
            let extension = entry
                .get("extension")
                .and_then(Value::as_str)
                .unwrap_or("[none]");
            *extension_counts.entry(extension.into()).or_default() += 1;
        }
        Ok(json!({
            "root": self.relative(&root, ambient)?,
            "files": entries,
            "file_count": entries.len(),
            "extension_counts": extension_counts,
            "truncated": truncated,
        }))
    }

    pub(super) fn symbol_search(
        &self,
        path: &str,
        pattern: &str,
        max_results: usize,
        ambient: bool,
    ) -> Result<Value, ExecutionError> {
        let root = self.resolve(path, ambient)?;
        if !root.is_dir() {
            return Err(ExecutionError::Failed(
                "repository symbol search path must be a directory".into(),
            ));
        }
        let maximum = max_results.clamp(1, 500);
        let (files, files_truncated) = self.files(&root, 5_000, ambient)?;
        let mut symbols = Vec::new();
        let mut truncated = files_truncated;
        'files: for file in files {
            let Some(content) = self.bounded_text(&file)? else {
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                let Some(mut symbol) = structural_symbol(line) else {
                    continue;
                };
                let matched = ["kind", "name", "text"].into_iter().any(|field| {
                    symbol
                        .get(field)
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.contains(pattern))
                });
                if !matched {
                    continue;
                }
                symbol["path"] = Value::String(self.relative(&file, ambient)?);
                symbol["line"] = json!(index + 1);
                symbols.push(symbol);
                if symbols.len() == maximum {
                    truncated = true;
                    break 'files;
                }
            }
        }
        Ok(json!({
            "query": pattern,
            "symbols": symbols,
            "match_count": symbols.len(),
            "truncated": truncated,
        }))
    }

    pub(super) fn search(
        &self,
        path: &str,
        needle: &str,
        max_results: usize,
        ambient: bool,
    ) -> Result<Value, ExecutionError> {
        let root = self.resolve(path, ambient)?;
        if !root.is_dir() {
            return Err(ExecutionError::Failed(
                "repository search path must be a directory".into(),
            ));
        }
        let maximum = max_results.clamp(1, 500);
        let (files, files_truncated) = self.files(&root, 5_000, ambient)?;
        let mut matches = Vec::new();
        let mut truncated = files_truncated;
        'files: for file in files {
            let Some(content) = self.bounded_text(&file)? else {
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                for offset in line.match_indices(needle).map(|(offset, _)| offset) {
                    if !token_match(line, offset, needle.len()) {
                        continue;
                    }
                    matches.push(json!({
                        "path": self.relative(&file, ambient)?,
                        "line": index + 1,
                        "column": offset + 1,
                        "text": bounded_tool_text(line.trim(), 400),
                    }));
                    if matches.len() == maximum {
                        truncated = true;
                        break 'files;
                    }
                }
            }
        }
        Ok(json!({
            "query": needle,
            "references": matches,
            "match_count": matches.len(),
            "truncated": truncated,
        }))
    }

    pub(super) fn file_summary(
        &self,
        path: &str,
        max_lines: usize,
        ambient: bool,
    ) -> Result<Value, ExecutionError> {
        let file = self.resolve(path, ambient)?;
        if !file.is_file() {
            return Err(ExecutionError::Failed(
                "repo.file_summary path must be a file".into(),
            ));
        }
        let content = self.bounded_text(&file)?.ok_or_else(|| {
            ExecutionError::Failed("repo.file_summary requires bounded UTF-8 text".into())
        })?;
        let line_count = content.lines().count();
        let selected_line_count = max_lines.clamp(1, 500);
        let preview_source = content
            .lines()
            .take(selected_line_count)
            .collect::<Vec<_>>()
            .join("\n");
        let (preview, preview_bytes_truncated) =
            bounded_json_string(&preview_source, MAX_FILE_SUMMARY_PREVIEW_JSON_BYTES);
        let symbols = bounded_json_collection(
            content.lines().filter_map(structural_symbol),
            200,
            MAX_FILE_SUMMARY_COLLECTION_JSON_BYTES,
        );
        let imports = bounded_json_collection(
            content
                .lines()
                .map(str::trim)
                .filter(|line| {
                    line.starts_with("import ")
                        || line.starts_with("from ")
                        || line.starts_with("use ")
                        || line.starts_with("mod ")
                        || line.starts_with("const ")
                        || line.starts_with("let ")
                        || line.starts_with("var ")
                })
                .map(|line| Value::String(bounded_tool_text(line, 500))),
            100,
            MAX_FILE_SUMMARY_COLLECTION_JSON_BYTES,
        );
        let headings = bounded_json_collection(
            content
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with('#'))
                .map(|line| Value::String(bounded_tool_text(line, 500))),
            100,
            MAX_FILE_SUMMARY_COLLECTION_JSON_BYTES,
        );
        let relative = self.relative(&file, ambient)?;
        let (path, path_truncated) =
            bounded_json_string(&relative, MAX_FILE_SUMMARY_PATH_JSON_BYTES);
        let summary = json!({
            "path": path,
            "path_truncated": path_truncated,
            "bytes": content.len(),
            "line_count": line_count,
            "extension": file.extension().and_then(|value| value.to_str()),
            "imports": imports,
            "headings": headings,
            "symbols": symbols,
            "preview": preview,
            "preview_truncated": line_count > selected_line_count || preview_bytes_truncated,
        });
        let output_bytes = serde_json::to_vec(&summary)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?
            .len();
        if output_bytes > MAX_FILE_SUMMARY_OUTPUT_BYTES {
            return Err(ExecutionError::Failed(
                "repo.file_summary exceeded its internal output bound".into(),
            ));
        }
        Ok(summary)
    }
}

fn bounded_json_string(text: &str, max_encoded_bytes: usize) -> (String, bool) {
    if serde_json::to_vec(text).is_ok_and(|encoded| encoded.len() <= max_encoded_bytes) {
        return (text.into(), false);
    }

    let mut boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    boundaries.push(text.len());
    let mut low = 0;
    let mut high = boundaries.len().saturating_sub(1);
    while low < high {
        let middle = (low + high).div_ceil(2);
        let end = boundaries[middle];
        if serde_json::to_vec(&text[..end]).is_ok_and(|encoded| encoded.len() <= max_encoded_bytes)
        {
            low = middle;
        } else {
            high = middle.saturating_sub(1);
        }
    }
    let end = boundaries[low];
    (text[..end].into(), end < text.len())
}

fn bounded_json_collection(
    values: impl IntoIterator<Item = Value>,
    max_items: usize,
    max_encoded_bytes: usize,
) -> Vec<Value> {
    let mut output = Vec::new();
    let mut encoded_bytes = 2_usize;
    for value in values.into_iter().take(max_items) {
        let Ok(encoded) = serde_json::to_vec(&value) else {
            break;
        };
        let separator = usize::from(!output.is_empty());
        let next_bytes = encoded_bytes
            .saturating_add(separator)
            .saturating_add(encoded.len());
        if next_bytes > max_encoded_bytes {
            break;
        }
        output.push(value);
        encoded_bytes = next_bytes;
    }
    output
}

#[async_trait]
impl EffectExecutor for RepositoryEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: RepositoryOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let ambient = permit.obligations().resource_authority == ResourceAuthority::Ambient;
        let expected_resource = self.resolve(operation.resource(), ambient)?;
        if request.action != operation.action()
            || Path::new(&request.resource) != expected_resource.as_path()
        {
            return Err(ExecutionError::Failed(
                "repository request does not match its validated operation".into(),
            ));
        }
        let value = match operation {
            RepositoryOperation::Map { path, max_files } => self.map(&path, max_files, ambient)?,
            RepositoryOperation::SymbolSearch {
                pattern,
                path,
                max_results,
            } => self.symbol_search(&path, &pattern, max_results, ambient)?,
            RepositoryOperation::References {
                symbol,
                path,
                max_results,
            } => self.search(&path, &symbol, max_results, ambient)?,
            RepositoryOperation::FileSummary { path, max_lines } => {
                self.file_summary(&path, max_lines, ambient)?
            }
        };
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}

pub(super) fn token_match(line: &str, offset: usize, length: usize) -> bool {
    let before = line[..offset].chars().next_back();
    let after = line[offset + length..].chars().next();
    !before.is_some_and(symbol_character) && !after.is_some_and(symbol_character)
}

pub(super) fn symbol_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

pub(super) fn structural_symbol(line: &str) -> Option<Value> {
    let trimmed = line.trim_start();
    let (prefix, kind) = [
        ("pub async fn ", "function"),
        ("async fn ", "function"),
        ("pub fn ", "function"),
        ("fn ", "function"),
        ("pub struct ", "struct"),
        ("struct ", "struct"),
        ("pub enum ", "enum"),
        ("enum ", "enum"),
        ("pub trait ", "trait"),
        ("trait ", "trait"),
        ("class ", "class"),
        ("def ", "function"),
        ("function ", "function"),
        ("interface ", "interface"),
        ("type ", "type"),
        ("pub const ", "constant"),
        ("const ", "constant"),
    ]
    .into_iter()
    .find(|(prefix, _)| trimmed.starts_with(prefix))?;
    let name = trimmed[prefix.len()..]
        .chars()
        .take_while(|character| symbol_character(*character) || *character == '$')
        .collect::<String>();
    if name.is_empty() {
        return None;
    }
    Some(json!({
        "kind": kind,
        "name": name,
        "text": bounded_tool_text(trimmed, 300),
    }))
}
