use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemeFile {
    schema_version: u16,
    name: String,
    #[serde(default)]
    base: ThemeName,
    title: Option<String>,
    caret: Option<String>,
    continuation: Option<String>,
    #[serde(default)]
    prompt: ThemePromptFile,
    #[serde(default)]
    styles: ThemeStylesFile,
    spinner: Option<ThemeSpinner>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemePromptFile {
    left: Option<String>,
    right: Option<String>,
    indicator: Option<String>,
    continuation: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemeStylesFile {
    assistant: Option<ThemeStyleFile>,
    activity: Option<ThemeStyleFile>,
    thinking: Option<ThemeStyleFile>,
    tool: Option<ThemeStyleFile>,
    success: Option<ThemeStyleFile>,
    warning: Option<ThemeStyleFile>,
    error: Option<ThemeStyleFile>,
    meta: Option<ThemeStyleFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemeStyleFile {
    foreground: Option<String>,
    bold: Option<bool>,
    dim: Option<bool>,
    italic: Option<bool>,
}

/// Bounded metadata for one loaded custom-theme library.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ThemeLibraryStatus {
    /// Deterministic built-in and custom names.
    pub names: Vec<String>,
    /// Configuration directories inspected in precedence order.
    pub directories: Vec<PathBuf>,
}

/// Validated custom-theme template that can be saved by an operator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ThemeScaffold {
    /// Canonical custom theme identity.
    pub name: String,
    /// Suggested config-adjacent destination.
    pub suggested_path: Option<PathBuf>,
    /// Template serialization format.
    pub format: String,
    /// Strict schema-v1 template content.
    pub content: String,
}

/// Strict data-only custom-theme library loaded during trusted configuration bootstrap.
#[derive(Clone, Debug, Default)]
pub struct ThemeLibrary {
    custom: BTreeMap<String, CustomTheme>,
    directories: Vec<PathBuf>,
}

impl ThemeLibrary {
    /// Load JSON and TOML themes from bounded, non-symlink configuration directories.
    pub fn load(directories: &[PathBuf]) -> Result<Self, PresentationError> {
        let mut library = Self {
            custom: BTreeMap::new(),
            directories: directories.to_vec(),
        };
        for directory in directories {
            let metadata = match fs::symlink_metadata(directory) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(PresentationError::Invalid(error.to_string())),
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PresentationError::Invalid(format!(
                    "theme library is not a real directory: {}",
                    directory.display()
                )));
            }
            let mut paths = fs::read_dir(directory)
                .map_err(|error| PresentationError::Invalid(error.to_string()))?
                .map(|entry| {
                    entry
                        .map(|entry| entry.path())
                        .map_err(|error| PresentationError::Invalid(error.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            paths.sort();
            for path in paths {
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|error| PresentationError::Invalid(error.to_string()))?;
                if metadata.file_type().is_symlink() {
                    return Err(PresentationError::Invalid(format!(
                        "theme files cannot be symlinks: {}",
                        path.display()
                    )));
                }
                let extension = path.extension().and_then(|value| value.to_str());
                if !matches!(extension, Some("json" | "toml")) {
                    continue;
                }
                if !metadata.is_file() || metadata.len() > MAX_THEME_FILE_BYTES {
                    return Err(PresentationError::Invalid(format!(
                        "theme file must be regular and at most {MAX_THEME_FILE_BYTES} bytes: {}",
                        path.display()
                    )));
                }
                if library.custom.len() == MAX_CUSTOM_THEMES {
                    return Err(PresentationError::Invalid(format!(
                        "custom theme count exceeds {MAX_CUSTOM_THEMES}"
                    )));
                }
                let bytes = read_bounded_theme_file(&path)?;
                let theme = parse_theme_file(&path, &bytes)?;
                if ThemeName::parse(&theme.name).is_some()
                    || library.custom.insert(theme.name.clone(), theme).is_some()
                {
                    return Err(PresentationError::Invalid(format!(
                        "theme identity is built-in or duplicated: {}",
                        path.display()
                    )));
                }
            }
        }
        Ok(library)
    }

    /// Load config-adjacent themes plus the platform user theme library.
    pub fn load_for_config(config_path: &Path) -> Result<Self, PresentationError> {
        let mut directories = Vec::new();
        if let Some(parent) = config_path.parent() {
            directories.push(parent.join("themes"));
        }
        let user_directory = if let Some(value) = std::env::var_os("COLOSSUS_THEME_DIR") {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(PresentationError::Invalid(
                    "COLOSSUS_THEME_DIR must be absolute".into(),
                ));
            }
            Some(path)
        } else {
            default_user_theme_directory()
        };
        if let Some(directory) = user_directory
            && !directories.contains(&directory)
        {
            directories.push(directory);
        }
        Self::load(&directories)
    }

    /// Deterministic library status without file contents.
    pub fn status(&self) -> ThemeLibraryStatus {
        ThemeLibraryStatus {
            names: self.names(),
            directories: self.directories.clone(),
        }
    }

    /// Build the human-first theme library view used by interactive terminal surfaces.
    pub fn status_document(&self, selected: &str) -> PresentationDocument {
        let mut themes = PresentationTable::new(
            ["#", "Active", "Theme", "Type", "Character"],
            "No terminal themes are available.",
        );
        for (index, name) in self.names().into_iter().enumerate() {
            themes.push_row([
                (index + 1).to_string(),
                if name == selected { "yes" } else { "" }.into(),
                name.clone(),
                if ThemeName::parse(&name).is_some() {
                    "Built-in"
                } else {
                    "Custom"
                }
                .into(),
                theme_character(&name).into(),
            ]);
        }

        let mut directories = PresentationTable::new(
            ["#", "Custom theme folder"],
            "No custom theme folders are configured.",
        );
        for (index, directory) in self.directories.iter().enumerate() {
            directories.push_row([(index + 1).to_string(), directory.display().to_string()]);
        }

        PresentationDocument {
            blocks: vec![
                PresentationBlock::Markdown(format!(
                    "# Themes\n\nActive theme: **{selected}**\n\nChoose a number or name to apply a theme. Use `p NUMBER` or `/theme preview NAME` to inspect one without changing your selection."
                )),
                PresentationBlock::Table(themes),
                PresentationBlock::Card {
                    title: "Custom theme search locations".into(),
                    tone: PresentationTone::Neutral,
                    body: vec![PresentationBlock::Table(directories)],
                },
            ],
        }
    }

    /// Build a complete visual sample for one theme without selecting it.
    pub fn preview_document(&self, name: &str) -> Result<PresentationDocument, PresentationError> {
        let theme = self.preview(name)?;
        let built_in = ThemeName::parse(&theme.name).is_some();
        let prompt_title = if built_in {
            "Colossus"
        } else {
            theme.title.as_str()
        };
        let mut metadata = PresentationTable::new(["Field", "Value"], "No theme metadata.");
        for (field, value) in [
            ("Name", theme.name.clone()),
            ("Type", if built_in { "Built-in" } else { "Custom" }.into()),
            ("Base", theme.base.as_str().into()),
            ("Spinner", format!("{:?}", theme.spinner)),
        ] {
            metadata.push_row([field, value.as_str()]);
        }
        if !built_in {
            metadata.push_row(["Source hash", theme.source_hash.as_str()]);
        }

        Ok(PresentationDocument {
            blocks: vec![
                PresentationBlock::Markdown(format!(
                    "# {} theme preview\n\nThis is a preview only; your active theme has not changed.",
                    human_field_name(&theme.name)
                )),
                PresentationBlock::Prompt {
                    left: format!("{prompt_title} 019f-theme"),
                    indicator: if built_in { "›".into() } else { theme.caret.clone() },
                    input: "Ask Colossus to review this change".into(),
                    right: Some("primary:openrouter status=ready".into()),
                },
                PresentationBlock::Table(metadata),
                PresentationBlock::Card {
                    title: "Assistant".into(),
                    tone: PresentationTone::Neutral,
                    body: vec![PresentationBlock::Markdown(
                        "## Markdown sample\n\nThe theme styles **answers**, `inline code`, links, and lists.\n\n- Clear hierarchy\n- Readable content".into(),
                    )],
                },
                PresentationBlock::Card {
                    title: "Thinking".into(),
                    tone: PresentationTone::Thinking,
                    body: vec![PresentationBlock::Markdown(
                        "_Reviewing the relevant files and constraints…_".into(),
                    )],
                },
                PresentationBlock::Card {
                    title: "Completed filesystem.read".into(),
                    tone: PresentationTone::Tool,
                    body: vec![PresentationBlock::KeyValue(vec![
                        ("Status".into(), "ok".into()),
                        ("Duration".into(), "0.42s".into()),
                    ])],
                },
                PresentationBlock::Card {
                    title: "Approval required".into(),
                    tone: PresentationTone::Warning,
                    body: vec![PresentationBlock::Text(
                        "This effect needs your confirmation before it runs.".into(),
                    )],
                },
                PresentationBlock::Card {
                    title: "Completed".into(),
                    tone: PresentationTone::Success,
                    body: vec![PresentationBlock::Text(
                        "The requested work finished successfully.".into(),
                    )],
                },
                PresentationBlock::Card {
                    title: "Needs attention".into(),
                    tone: PresentationTone::Error,
                    body: vec![PresentationBlock::Text(
                        "A denied or failed effect is visually distinct.".into(),
                    )],
                },
                PresentationBlock::Diff(
                    "@@ -1,2 +1,2 @@\n-old terminal output\n+human-first terminal output".into(),
                ),
            ],
        })
    }

    /// Resolve temporary preferences for a visual preview without mutating the caller.
    pub fn preview_preferences(
        &self,
        name: &str,
        current: &TerminalPreferences,
    ) -> Result<TerminalPreferences, PresentationError> {
        let mut preview = current.clone();
        self.select(name, &mut preview)?;
        Ok(preview)
    }

    /// Build a human validation summary for the already loaded strict library.
    pub fn validation_document(&self) -> PresentationDocument {
        let custom_count = self.custom.len();
        PresentationDocument::from_block(PresentationBlock::Card {
            title: "Theme library valid".into(),
            tone: PresentationTone::Success,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Built-in themes".into(), "5".into()),
                    ("Custom themes".into(), custom_count.to_string()),
                    ("Maximum custom themes".into(), MAX_CUSTOM_THEMES.to_string()),
                    (
                        "Maximum file size".into(),
                        format!("{} KiB", MAX_THEME_FILE_BYTES / 1024),
                    ),
                ]),
                PresentationBlock::Markdown(
                    "Every discovered JSON/TOML theme passed the strict schema, identity, color, size, collision, and symlink checks.".into(),
                ),
            ],
        })
    }

    /// Build a concise confirmation after a theme preference is durably saved.
    pub fn selection_document(&self, selected: &str) -> PresentationDocument {
        PresentationDocument::from_block(PresentationBlock::Card {
            title: "Theme applied".into(),
            tone: PresentationTone::Success,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Theme".into(), selected.into()),
                    ("Saved".into(), "yes".into()),
                ]),
                PresentationBlock::Markdown(format!(
                    "Use `/theme preview {selected}` for the complete visual sample."
                )),
            ],
        })
    }

    /// Produce a strict TOML starter without writing through the terminal interface.
    pub fn scaffold(&self, name: &str) -> Result<ThemeScaffold, PresentationError> {
        let name = normalize_theme_name(name)?;
        if ThemeName::parse(&name).is_some() || self.custom.contains_key(&name) {
            return Err(PresentationError::Invalid(format!(
                "theme identity already exists: {name}"
            )));
        }
        let content = format!(
            "schemaVersion = 1\nname = \"{name}\"\nbase = \"default\"\ntitle = \"Colossus\"\ncaret = \"›\"\ncontinuation = \"…\"\nspinner = \"dots\"\n\n[prompt]\nleft = \"#5FD7FF\"\nright = \"#7F8790\"\nindicator = \"#5FD7FF\"\ncontinuation = \"#7F8790\"\n\n[styles.tool]\nforeground = \"#58A6FF\"\nbold = true\n\n[styles.meta]\nforeground = \"#7F8790\"\ndim = true\n"
        );
        Ok(ThemeScaffold {
            suggested_path: self
                .directories
                .first()
                .map(|directory| directory.join(format!("{name}.toml"))),
            name,
            format: "toml".into(),
            content,
        })
    }

    /// Render one scaffold with explicit no-write and restart guidance.
    pub fn scaffold_document(scaffold: &ThemeScaffold) -> PresentationDocument {
        let destination = scaffold.suggested_path.as_ref().map_or_else(
            || "a configured theme folder".into(),
            |path| path.display().to_string(),
        );
        PresentationDocument::from_block(PresentationBlock::Card {
            title: format!("Custom theme scaffold: {}", scaffold.name),
            tone: PresentationTone::Neutral,
            body: vec![
                PresentationBlock::KeyValue(vec![
                    ("Suggested path".into(), destination),
                    ("Format".into(), scaffold.format.clone()),
                ]),
                PresentationBlock::Code {
                    language: Some("toml".into()),
                    content: scaffold.content.clone(),
                },
                PresentationBlock::Markdown(
                    "The TUI does **not** write this file. Save it deliberately, restart Colossus, then run `/theme validate` and `/theme NAME`.".into(),
                ),
            ],
        })
    }

    /// Deterministic built-in and custom identities.
    pub fn names(&self) -> Vec<String> {
        ["default", "mono", "high_contrast", "carrot", "hacker"]
            .into_iter()
            .map(str::to_owned)
            .chain(self.custom.keys().cloned())
            .collect()
    }

    /// Resolve one immutable preview snapshot without changing preferences.
    pub fn preview(&self, name: &str) -> Result<CustomTheme, PresentationError> {
        let normalized = normalize_theme_name(name)?;
        if let Some(theme) = ThemeName::parse(&normalized) {
            return Ok(builtin_snapshot(theme));
        }
        self.custom.get(&normalized).cloned().ok_or_else(|| {
            PresentationError::Invalid(format!(
                "unknown theme {name}; available={}",
                self.names().join(",")
            ))
        })
    }

    /// Select one built-in or immutable custom snapshot.
    pub fn select(
        &self,
        name: &str,
        preferences: &mut TerminalPreferences,
    ) -> Result<(), PresentationError> {
        let normalized = normalize_theme_name(name)?;
        if let Some(theme) = ThemeName::parse(&normalized) {
            preferences.select_builtin_theme(theme);
            return Ok(());
        }
        let theme = self.custom.get(&normalized).cloned().ok_or_else(|| {
            PresentationError::Invalid(format!(
                "unknown theme {name}; available={}",
                self.names().join(",")
            ))
        })?;
        preferences.select_custom_theme(theme);
        Ok(())
    }
}

fn theme_character(name: &str) -> &'static str {
    match ThemeName::parse(name) {
        Some(ThemeName::Default) => "Balanced blue",
        Some(ThemeName::Mono) => "Color-free",
        Some(ThemeName::HighContrast) => "Strong contrast",
        Some(ThemeName::Carrot) => "Warm orange",
        Some(ThemeName::Hacker) => "Green terminal",
        None => "Custom palette",
    }
}

/// Platform-standard Colossus user theme directory.
pub fn default_user_theme_directory() -> Option<PathBuf> {
    BaseDirs::new().map(|directories| directories.config_dir().join("colossus").join("themes"))
}

fn read_bounded_theme_file(path: &Path) -> Result<Vec<u8>, PresentationError> {
    let file = fs::File::open(path)
        .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?;
    let metadata = file
        .metadata()
        .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?;
    if !metadata.is_file() || metadata.len() > MAX_THEME_FILE_BYTES {
        return Err(PresentationError::Invalid(format!(
            "theme file must be regular and at most {MAX_THEME_FILE_BYTES} bytes: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_THEME_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?;
    if bytes.len() as u64 > MAX_THEME_FILE_BYTES {
        return Err(PresentationError::Invalid(format!(
            "theme file exceeds {MAX_THEME_FILE_BYTES} bytes: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

fn parse_theme_file(path: &Path, bytes: &[u8]) -> Result<CustomTheme, PresentationError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?;
    let file: ThemeFile = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?,
        Some("toml") => toml::from_str(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?,
        _ => {
            return Err(PresentationError::Invalid(
                "unsupported theme extension".into(),
            ));
        }
    };
    if file.schema_version != 1 {
        return Err(PresentationError::Invalid(format!(
            "theme schemaVersion must be 1: {}",
            path.display()
        )));
    }
    let name = normalize_theme_name(&file.name)?;
    let base = builtin_snapshot(file.base);
    Ok(CustomTheme {
        schema_version: 1,
        name,
        source_hash: hex::encode(Sha256::digest(bytes)),
        base: file.base,
        title: bounded_theme_text(file.title.as_deref().unwrap_or("colossus"), 32, "title")?,
        caret: bounded_theme_text(file.caret.as_deref().unwrap_or(">"), 8, "caret")?,
        continuation: bounded_theme_text(
            file.continuation.as_deref().unwrap_or("|"),
            8,
            "continuation",
        )?,
        prompt_left: resolve_color(file.prompt.left.as_deref(), base.prompt_left)?,
        prompt_right: resolve_color(file.prompt.right.as_deref(), base.prompt_right)?,
        indicator: resolve_color(file.prompt.indicator.as_deref(), base.indicator)?,
        continuation_color: resolve_color(
            file.prompt.continuation.as_deref(),
            base.continuation_color,
        )?,
        assistant: resolve_style(base.assistant, file.styles.assistant.as_ref())?,
        activity: resolve_style(base.activity, file.styles.activity.as_ref())?,
        thinking: resolve_style(base.thinking, file.styles.thinking.as_ref())?,
        tool: resolve_style(base.tool, file.styles.tool.as_ref())?,
        success: resolve_style(base.success, file.styles.success.as_ref())?,
        warning: resolve_style(base.warning, file.styles.warning.as_ref())?,
        error: resolve_style(base.error, file.styles.error.as_ref())?,
        meta: resolve_style(base.meta, file.styles.meta.as_ref())?,
        spinner: file.spinner.unwrap_or(base.spinner),
    })
}

fn normalize_theme_name(value: &str) -> Result<String, PresentationError> {
    let value = value.trim().to_ascii_lowercase().replace('-', "_");
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(PresentationError::Invalid(
            "theme name must be a simple 1-64 character identifier".into(),
        ));
    }
    Ok(value)
}

fn bounded_theme_text(
    value: &str,
    max_chars: usize,
    field: &str,
) -> Result<String, PresentationError> {
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        return Err(PresentationError::Invalid(format!(
            "theme {field} must be nonempty, control-free, and at most {max_chars} characters"
        )));
    }
    Ok(value.into())
}

fn resolve_color(
    value: Option<&str>,
    base: Option<ThemeColor>,
) -> Result<Option<ThemeColor>, PresentationError> {
    value
        .map(parse_color)
        .transpose()
        .map(|value| value.or(base))
}

fn parse_color(value: &str) -> Result<ThemeColor, PresentationError> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(PresentationError::Invalid(format!(
            "theme colors must use #RRGGBB: {value}"
        )));
    }
    Ok(ThemeColor {
        red: u8::from_str_radix(&value[1..3], 16)
            .map_err(|error| PresentationError::Invalid(error.to_string()))?,
        green: u8::from_str_radix(&value[3..5], 16)
            .map_err(|error| PresentationError::Invalid(error.to_string()))?,
        blue: u8::from_str_radix(&value[5..7], 16)
            .map_err(|error| PresentationError::Invalid(error.to_string()))?,
    })
}

fn resolve_style(
    base: ThemeTextStyle,
    value: Option<&ThemeStyleFile>,
) -> Result<ThemeTextStyle, PresentationError> {
    let Some(value) = value else {
        return Ok(base);
    };
    Ok(ThemeTextStyle {
        foreground: resolve_color(value.foreground.as_deref(), base.foreground)?,
        bold: value.bold.unwrap_or(base.bold),
        dim: value.dim.unwrap_or(base.dim),
        italic: value.italic.unwrap_or(base.italic),
    })
}

fn builtin_snapshot(theme: ThemeName) -> CustomTheme {
    let palette = TerminalPalette::for_theme(theme);
    let identity = theme.as_str();
    CustomTheme {
        schema_version: 1,
        name: identity.into(),
        source_hash: hex::encode(Sha256::digest(format!("builtin:{identity}").as_bytes())),
        base: theme,
        title: "colossus".into(),
        caret: "›".into(),
        continuation: "…".into(),
        prompt_left: palette.prompt_left.map(Into::into),
        prompt_right: palette.prompt_right.map(Into::into),
        indicator: palette.indicator.map(Into::into),
        continuation_color: palette.continuation.map(Into::into),
        assistant: palette.assistant.into(),
        activity: palette.activity.into(),
        thinking: palette.thinking.into(),
        tool: palette.tool.into(),
        success: palette.success.into(),
        warning: palette.warning.into(),
        error: palette.error.into(),
        meta: palette.meta.into(),
        spinner: match theme {
            ThemeName::Default => ThemeSpinner::Dots,
            ThemeName::Mono => ThemeSpinner::Line,
            ThemeName::HighContrast => ThemeSpinner::Arc,
            ThemeName::Carrot => ThemeSpinner::BouncingBar,
            ThemeName::Hacker => ThemeSpinner::Aesthetic,
        },
    }
}

pub(super) fn validate_preferences(preferences: &TerminalPreferences) -> Result<(), StoreError> {
    if preferences.schema_version != 1 {
        return Err(StoreError::Adapter("schema_version must be 1".into()));
    }
    if let Some(theme) = &preferences.custom_theme
        && (theme.schema_version != 1
            || theme.base != preferences.theme
            || normalize_theme_name(&theme.name).is_err()
            || theme.name.contains('-')
            || ThemeName::parse(&theme.name).is_some()
            || theme.source_hash.len() != 64
            || !theme
                .source_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || bounded_theme_text(&theme.title, 32, "title").is_err()
            || bounded_theme_text(&theme.caret, 8, "caret").is_err()
            || bounded_theme_text(&theme.continuation, 8, "continuation").is_err())
    {
        return Err(StoreError::Adapter(
            "custom theme snapshot is invalid or inconsistent".into(),
        ));
    }
    Ok(())
}
