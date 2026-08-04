use super::*;

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
    pub(super) fn resolve(&self, relative: &str) -> Result<PathBuf, ExecutionError> {
        let requested = Path::new(relative);
        if relative.contains('\0')
            || requested.is_absolute()
            || requested.components().any(|component| {
                matches!(component, std::path::Component::ParentDir)
                    || matches!(component.as_os_str().to_str(), Some(".git" | ".colossus"))
            })
        {
            return Err(ExecutionError::Failed(
                "repository paths must remain inside the workspace and outside control state"
                    .into(),
            ));
        }
        let joined = self.workspace.join(requested);
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
        if !canonical.starts_with(&self.workspace) {
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
    ) -> Result<(Vec<PathBuf>, bool), ExecutionError> {
        let mut files = Vec::new();
        let mut truncated = false;
        let hard_limit = maximum.clamp(1, 5_000);
        let walker = WalkBuilder::new(root)
            .follow_links(false)
            .hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .parents(false)
            .build();
        for entry in walker {
            let entry = entry.map_err(|error| ExecutionError::Failed(error.to_string()))?;
            let relative = entry.path().strip_prefix(&self.workspace).map_err(|_| {
                ExecutionError::Failed("repository walk escaped the active workspace".into())
            })?;
            if relative.components().any(|component| {
                matches!(component.as_os_str().to_str(), Some(".git" | ".colossus"))
            }) {
                continue;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let canonical = fs::canonicalize(entry.path())
                .map_err(|error| ExecutionError::Failed(error.to_string()))?;
            if !canonical.starts_with(&self.workspace) {
                return Err(ExecutionError::Failed(
                    "repository walk escaped the active workspace".into(),
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

    pub(super) fn relative(&self, path: &Path) -> Result<String, ExecutionError> {
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

    pub(super) fn map(&self, path: &str, max_files: usize) -> Result<Value, ExecutionError> {
        let root = self.resolve(path)?;
        if !root.is_dir() {
            return Err(ExecutionError::Failed(
                "repo.map path must be a directory".into(),
            ));
        }
        let (files, truncated) = self.files(&root, max_files.clamp(1, 1_000))?;
        let entries = files
            .iter()
            .map(|file| {
                let metadata = fs::metadata(file)
                    .map_err(|error| ExecutionError::Failed(error.to_string()))?;
                Ok(json!({
                    "path": self.relative(file)?,
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
            "root": self.relative(&root)?,
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
    ) -> Result<Value, ExecutionError> {
        let root = self.resolve(path)?;
        if !root.is_dir() {
            return Err(ExecutionError::Failed(
                "repository symbol search path must be a directory".into(),
            ));
        }
        let maximum = max_results.clamp(1, 500);
        let (files, files_truncated) = self.files(&root, 5_000)?;
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
                symbol["path"] = Value::String(self.relative(&file)?);
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
    ) -> Result<Value, ExecutionError> {
        let root = self.resolve(path)?;
        if !root.is_dir() {
            return Err(ExecutionError::Failed(
                "repository search path must be a directory".into(),
            ));
        }
        let maximum = max_results.clamp(1, 500);
        let (files, files_truncated) = self.files(&root, 5_000)?;
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
                        "path": self.relative(&file)?,
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
    ) -> Result<Value, ExecutionError> {
        let file = self.resolve(path)?;
        if !file.is_file() {
            return Err(ExecutionError::Failed(
                "repo.file_summary path must be a file".into(),
            ));
        }
        let content = self.bounded_text(&file)?.ok_or_else(|| {
            ExecutionError::Failed("repo.file_summary requires bounded UTF-8 text".into())
        })?;
        let line_count = content.lines().count();
        let preview = content
            .lines()
            .take(max_lines.clamp(1, 500))
            .collect::<Vec<_>>()
            .join("\n");
        let symbols = content
            .lines()
            .filter_map(structural_symbol)
            .take(200)
            .collect::<Vec<_>>();
        let imports = content
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
            .take(100)
            .map(|line| bounded_tool_text(line, 500))
            .collect::<Vec<_>>();
        let headings = content
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('#'))
            .take(100)
            .map(|line| bounded_tool_text(line, 500))
            .collect::<Vec<_>>();
        Ok(json!({
            "path": self.relative(&file)?,
            "bytes": content.len(),
            "line_count": line_count,
            "extension": file.extension().and_then(|value| value.to_str()),
            "imports": imports,
            "headings": headings,
            "symbols": symbols,
            "preview": preview,
            "preview_truncated": line_count > max_lines.clamp(1, 500),
        }))
    }
}

#[async_trait]
impl EffectExecutor for RepositoryEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: RepositoryOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let expected_resource = self.resolve(operation.resource())?;
        if request.action != operation.action()
            || Path::new(&request.resource) != expected_resource.as_path()
        {
            return Err(ExecutionError::Failed(
                "repository request does not match its validated operation".into(),
            ));
        }
        let value = match operation {
            RepositoryOperation::Map { path, max_files } => self.map(&path, max_files)?,
            RepositoryOperation::SymbolSearch {
                pattern,
                path,
                max_results,
            } => self.symbol_search(&path, &pattern, max_results)?,
            RepositoryOperation::References {
                symbol,
                path,
                max_results,
            } => self.search(&path, &symbol, max_results)?,
            RepositoryOperation::FileSummary { path, max_lines } => {
                self.file_summary(&path, max_lines)?
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
