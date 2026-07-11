//! Event-sourced presentation preferences and pure semantic terminal rendering.

use colossus_contracts::{
    Actor, ContextStatus, CustomTheme, EventClassification, ExecutionContext, NewEvent,
    ProviderEvent, RunEvent, RunEventEnvelope, RunPhase, ThemeColor, ThemeSpinner, ThemeTextStyle,
    ToolCall, ToolResult, WorkStateSnapshot,
};
pub use colossus_contracts::{
    EventDisplayMode, ReplPreferences, StreamDisplayMode, ThemeName, TranscriptDensity,
};
use colossus_ports::{EventJournal, PresentationRepository, StoreError};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

const PREFERENCES_STREAM: &str = "presentation:repl";
const PREFERENCES_UPDATED: &str = "presentation.preferences.updated.v1";
const HISTORY_STREAM: &str = "presentation:history";
const HISTORY_APPENDED: &str = "presentation.history.appended.v1";
const MAX_HISTORY_ENTRIES: usize = 1_000;
const MAX_HISTORY_ENTRY_BYTES: usize = 1024 * 1024;
const COMPACT_PREVIEW_CHARS: usize = 240;
const VERBOSE_PREVIEW_CHARS: usize = 8 * 1024;
const MAX_CUSTOM_THEMES: usize = 64;
const MAX_THEME_FILE_BYTES: u64 = 64 * 1024;

/// Presentation rendering failure.
#[derive(Debug, Error)]
pub enum PresentationError {
    /// Released content could not be rendered safely.
    #[error("presentation rendering failed: {0}")]
    Invalid(String),
}

/// Terminal RGB value shared by Reedline prompts and semantic transcript palettes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RgbColor {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

impl RgbColor {
    const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

impl From<ThemeColor> for RgbColor {
    fn from(color: ThemeColor) -> Self {
        Self::new(color.red, color.green, color.blue)
    }
}

impl From<RgbColor> for ThemeColor {
    fn from(color: RgbColor) -> Self {
        Self {
            red: color.red,
            green: color.green,
            blue: color.blue,
        }
    }
}

#[derive(Clone, Copy)]
struct TextStyle {
    foreground: Option<RgbColor>,
    bold: bool,
    dim: bool,
    italic: bool,
}

impl TextStyle {
    const fn color(foreground: RgbColor) -> Self {
        Self {
            foreground: Some(foreground),
            bold: false,
            dim: false,
            italic: false,
        }
    }

    const fn plain() -> Self {
        Self {
            foreground: None,
            bold: false,
            dim: false,
            italic: false,
        }
    }

    const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    fn paint(self, text: &str, enabled: bool) -> String {
        if !enabled {
            return text.into();
        }
        let mut codes = Vec::with_capacity(4);
        if self.bold {
            codes.push("1".into());
        }
        if self.dim {
            codes.push("2".into());
        }
        if self.italic {
            codes.push("3".into());
        }
        if let Some(color) = self.foreground {
            codes.push(format!("38;2;{};{};{}", color.red, color.green, color.blue));
        }
        if codes.is_empty() {
            text.into()
        } else {
            format!("\x1b[{}m{text}\x1b[0m", codes.join(";"))
        }
    }
}

impl From<ThemeTextStyle> for TextStyle {
    fn from(style: ThemeTextStyle) -> Self {
        Self {
            foreground: style.foreground.map(Into::into),
            bold: style.bold,
            dim: style.dim,
            italic: style.italic,
        }
    }
}

impl From<TextStyle> for ThemeTextStyle {
    fn from(style: TextStyle) -> Self {
        Self {
            foreground: style.foreground.map(Into::into),
            bold: style.bold,
            dim: style.dim,
            italic: style.italic,
        }
    }
}

/// Complete built-in data-only palette for one terminal theme.
#[derive(Clone, Copy)]
pub struct TerminalPalette {
    prompt_left: Option<RgbColor>,
    prompt_right: Option<RgbColor>,
    indicator: Option<RgbColor>,
    continuation: Option<RgbColor>,
    assistant: TextStyle,
    activity: TextStyle,
    thinking: TextStyle,
    tool: TextStyle,
    success: TextStyle,
    warning: TextStyle,
    error: TextStyle,
    meta: TextStyle,
    spinner_frames: &'static [&'static str],
}

impl TerminalPalette {
    /// Resolve one strict built-in palette.
    pub const fn for_theme(theme: ThemeName) -> Self {
        match theme {
            ThemeName::Default => Self {
                prompt_left: Some(RgbColor::new(95, 215, 255)),
                prompt_right: Some(RgbColor::new(127, 135, 144)),
                indicator: Some(RgbColor::new(95, 215, 255)),
                continuation: Some(RgbColor::new(127, 135, 144)),
                assistant: TextStyle::color(RgbColor::new(230, 237, 243)),
                activity: TextStyle::color(RgbColor::new(127, 135, 144)).dim(),
                thinking: TextStyle::color(RgbColor::new(95, 215, 255)).italic(),
                tool: TextStyle::color(RgbColor::new(88, 166, 255)).bold(),
                success: TextStyle::color(RgbColor::new(158, 206, 106)),
                warning: TextStyle::color(RgbColor::new(255, 223, 93)).bold(),
                error: TextStyle::color(RgbColor::new(255, 95, 95)).bold(),
                meta: TextStyle::color(RgbColor::new(127, 135, 144)).dim(),
                spinner_frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            },
            ThemeName::Mono => Self {
                prompt_left: None,
                prompt_right: None,
                indicator: None,
                continuation: None,
                assistant: TextStyle::plain(),
                activity: TextStyle::plain().dim(),
                thinking: TextStyle::plain().italic().dim(),
                tool: TextStyle::plain().bold(),
                success: TextStyle::plain(),
                warning: TextStyle::plain().bold(),
                error: TextStyle::plain().bold(),
                meta: TextStyle::plain().dim(),
                spinner_frames: &["-", "\\", "|", "/"],
            },
            ThemeName::HighContrast => Self {
                prompt_left: Some(RgbColor::new(255, 255, 0)),
                prompt_right: Some(RgbColor::new(255, 255, 255)),
                indicator: Some(RgbColor::new(255, 255, 0)),
                continuation: Some(RgbColor::new(255, 255, 0)),
                assistant: TextStyle::color(RgbColor::new(255, 255, 255)),
                activity: TextStyle::color(RgbColor::new(255, 255, 255)).bold(),
                thinking: TextStyle::color(RgbColor::new(255, 255, 0)).bold(),
                tool: TextStyle::color(RgbColor::new(0, 255, 255)).bold(),
                success: TextStyle::color(RgbColor::new(255, 255, 255)).bold(),
                warning: TextStyle::color(RgbColor::new(255, 255, 0)).bold(),
                error: TextStyle::color(RgbColor::new(255, 0, 0)).bold(),
                meta: TextStyle::color(RgbColor::new(255, 255, 255)),
                spinner_frames: &["◜", "◠", "◝", "◞", "◡", "◟"],
            },
            ThemeName::Carrot => Self {
                prompt_left: Some(RgbColor::new(255, 175, 95)),
                prompt_right: Some(RgbColor::new(184, 139, 106)),
                indicator: Some(RgbColor::new(255, 135, 0)),
                continuation: Some(RgbColor::new(215, 135, 95)),
                assistant: TextStyle::color(RgbColor::new(255, 240, 223)),
                activity: TextStyle::color(RgbColor::new(184, 139, 106)).dim(),
                thinking: TextStyle::color(RgbColor::new(255, 175, 95)).italic(),
                tool: TextStyle::color(RgbColor::new(255, 135, 0)).bold(),
                success: TextStyle::color(RgbColor::new(159, 215, 122)),
                warning: TextStyle::color(RgbColor::new(255, 223, 93)).bold(),
                error: TextStyle::color(RgbColor::new(255, 95, 95)).bold(),
                meta: TextStyle::color(RgbColor::new(184, 139, 106)).dim(),
                spinner_frames: &[
                    "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█", "▉", "▊", "▋", "▌", "▍", "▎",
                ],
            },
            ThemeName::Hacker => Self {
                prompt_left: Some(RgbColor::new(0, 255, 102)),
                prompt_right: Some(RgbColor::new(63, 191, 106)),
                indicator: Some(RgbColor::new(0, 255, 102)),
                continuation: Some(RgbColor::new(0, 170, 68)),
                assistant: TextStyle::color(RgbColor::new(215, 255, 215)),
                activity: TextStyle::color(RgbColor::new(63, 191, 106)).dim(),
                thinking: TextStyle::color(RgbColor::new(0, 255, 102)).italic(),
                tool: TextStyle::color(RgbColor::new(0, 215, 255)).bold(),
                success: TextStyle::color(RgbColor::new(0, 255, 102)),
                warning: TextStyle::color(RgbColor::new(255, 215, 95)).bold(),
                error: TextStyle::color(RgbColor::new(255, 95, 95)).bold(),
                meta: TextStyle::color(RgbColor::new(63, 191, 106)).dim(),
                spinner_frames: &["░", "▒", "▓", "█", "▓", "▒"],
            },
        }
    }

    /// Resolve a built-in palette or an immutable custom snapshot.
    pub fn for_preferences(preferences: &ReplPreferences) -> Self {
        preferences
            .custom_theme
            .as_ref()
            .map_or_else(|| Self::for_theme(preferences.theme), Self::for_custom)
    }

    fn for_custom(theme: &CustomTheme) -> Self {
        Self {
            prompt_left: theme.prompt_left.map(Into::into),
            prompt_right: theme.prompt_right.map(Into::into),
            indicator: theme.indicator.map(Into::into),
            continuation: theme.continuation_color.map(Into::into),
            assistant: theme.assistant.into(),
            activity: theme.activity.into(),
            thinking: theme.thinking.into(),
            tool: theme.tool.into(),
            success: theme.success.into(),
            warning: theme.warning.into(),
            error: theme.error.into(),
            meta: theme.meta.into(),
            spinner_frames: spinner_frames(theme.spinner),
        }
    }

    /// Left-prompt color, or reset for the mono palette.
    pub const fn prompt_left(self) -> Option<RgbColor> {
        self.prompt_left
    }

    /// Right-prompt color, or reset for the mono palette.
    pub const fn prompt_right(self) -> Option<RgbColor> {
        self.prompt_right
    }

    /// Primary prompt-indicator color, or reset for the mono palette.
    pub const fn indicator(self) -> Option<RgbColor> {
        self.indicator
    }

    /// Continuation-indicator color, or reset for the mono palette.
    pub const fn continuation(self) -> Option<RgbColor> {
        self.continuation
    }

    /// Render the theme's bounded spinner frame for one elapsed duration.
    pub fn activity_frame(self, elapsed_seconds: f64, color: bool) -> String {
        let index = ((elapsed_seconds.max(0.0) * 10.0) as usize) % self.spinner_frames.len();
        self.activity.paint(self.spinner_frames[index], color)
    }
}

const fn spinner_frames(spinner: ThemeSpinner) -> &'static [&'static str] {
    match spinner {
        ThemeSpinner::Dots => &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
        ThemeSpinner::Line => &["-", "\\", "|", "/"],
        ThemeSpinner::Arc => &["◜", "◠", "◝", "◞", "◡", "◟"],
        ThemeSpinner::BouncingBar => &[
            "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█", "▉", "▊", "▋", "▌", "▍", "▎",
        ],
        ThemeSpinner::Aesthetic => &["░", "▒", "▓", "█", "▓", "▒"],
    }
}

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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyThemeFile {
    name: Option<String>,
    title: Option<String>,
    caret: Option<String>,
    continuation: Option<String>,
    #[serde(default)]
    styles: BTreeMap<String, String>,
    #[serde(default, alias = "trace_styles")]
    trace: BTreeMap<String, String>,
    #[serde(default, alias = "transcript_styles")]
    transcript: BTreeMap<String, String>,
}

const LEGACY_STYLE_KEYS: &[&str] = &[
    "prompt.band",
    "prompt.title",
    "prompt.badge",
    "prompt.model",
    "prompt.caret",
    "prompt.rprompt",
    "prompt.continuation",
    "bottom-toolbar",
    "bottom-toolbar.key",
    "bottom-toolbar.warn",
];
const LEGACY_TRACE_KEYS: &[&str] = &[
    "thinking",
    "done",
    "tool_call",
    "tool_result",
    "approval_requested",
    "approval_auto_granted",
    "risk_assessment",
    "research",
    "context",
];
const LEGACY_TRANSCRIPT_KEYS: &[&str] = &[
    "user",
    "assistant",
    "reasoning",
    "tool",
    "tool_output",
    "approval",
    "risk",
    "research",
    "context",
    "error",
    "meta",
    "border",
    "activity_spinner",
];

/// Bounded metadata for one loaded custom-theme library.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ThemeLibraryStatus {
    /// Deterministic built-in and custom names.
    pub names: Vec<String>,
    /// Configuration directories inspected in precedence order.
    pub directories: Vec<PathBuf>,
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
        preferences: &mut ReplPreferences,
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
    let versioned = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str::<Value>(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?
            .get("schemaVersion")
            .is_some(),
        Some("toml") => toml::from_str::<toml::Value>(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?
            .get("schemaVersion")
            .is_some(),
        _ => {
            return Err(PresentationError::Invalid(
                "unsupported theme extension".into(),
            ));
        }
    };
    if !versioned {
        let file: LegacyThemeFile = match path.extension().and_then(|value| value.to_str()) {
            Some("json") => serde_json::from_str(text).map_err(|error| {
                PresentationError::Invalid(format!("{}: {error}", path.display()))
            })?,
            Some("toml") => toml::from_str(text).map_err(|error| {
                PresentationError::Invalid(format!("{}: {error}", path.display()))
            })?,
            _ => unreachable!("validated extension"),
        };
        return resolve_legacy_theme(path, bytes, file);
    }
    let file: ThemeFile = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?,
        Some("toml") => toml::from_str(text)
            .map_err(|error| PresentationError::Invalid(format!("{}: {error}", path.display())))?,
        _ => unreachable!("validated extension"),
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

fn resolve_legacy_theme(
    path: &Path,
    bytes: &[u8],
    file: LegacyThemeFile,
) -> Result<CustomTheme, PresentationError> {
    validate_legacy_keys(&file.styles, LEGACY_STYLE_KEYS, "style")?;
    validate_legacy_keys(&file.trace, LEGACY_TRACE_KEYS, "trace")?;
    validate_legacy_keys(&file.transcript, LEGACY_TRANSCRIPT_KEYS, "transcript")?;
    let base = builtin_snapshot(ThemeName::Default);
    let name = normalize_theme_name(
        file.name
            .as_deref()
            .or_else(|| path.file_stem().and_then(|value| value.to_str()))
            .ok_or_else(|| PresentationError::Invalid("theme file has no UTF-8 name".into()))?,
    )?;
    let prompt_color = |key: &str, fallback: Option<ThemeColor>| {
        file.styles
            .get(key)
            .map(|value| rich_foreground(value, fallback))
            .transpose()
            .map(|value| value.flatten().or(fallback))
    };
    Ok(CustomTheme {
        schema_version: 1,
        name,
        source_hash: hex::encode(Sha256::digest(bytes)),
        base: ThemeName::Default,
        title: bounded_theme_text(file.title.as_deref().unwrap_or("colossus"), 32, "title")?,
        caret: bounded_theme_text(file.caret.as_deref().unwrap_or(">"), 8, "caret")?,
        continuation: bounded_theme_text(
            file.continuation.as_deref().unwrap_or("|"),
            8,
            "continuation",
        )?,
        prompt_left: prompt_color("prompt.title", base.prompt_left)?,
        prompt_right: prompt_color("prompt.rprompt", base.prompt_right)?,
        indicator: prompt_color("prompt.caret", base.indicator)?,
        continuation_color: prompt_color("prompt.continuation", base.continuation_color)?,
        assistant: legacy_style(&file.transcript, &["assistant"], base.assistant)?,
        activity: legacy_style(&file.transcript, &["meta"], base.activity)?,
        thinking: legacy_style_maps(
            &[
                (&file.trace, &["thinking"]),
                (&file.transcript, &["reasoning"]),
            ],
            base.thinking,
        )?,
        tool: legacy_style_maps(
            &[(&file.trace, &["tool_call"]), (&file.transcript, &["tool"])],
            base.tool,
        )?,
        success: legacy_style(&file.trace, &["done", "tool_result"], base.success)?,
        warning: legacy_style_maps(
            &[
                (&file.trace, &["approval_requested"]),
                (&file.transcript, &["approval"]),
            ],
            base.warning,
        )?,
        error: legacy_style_maps(
            &[
                (&file.trace, &["risk_assessment"]),
                (&file.transcript, &["error", "risk"]),
            ],
            base.error,
        )?,
        meta: legacy_style(&file.transcript, &["meta", "context"], base.meta)?,
        spinner: file
            .transcript
            .get("activity_spinner")
            .map(|value| parse_spinner(value))
            .transpose()?
            .unwrap_or(base.spinner),
    })
}

fn validate_legacy_keys(
    values: &BTreeMap<String, String>,
    allowed: &[&str],
    family: &str,
) -> Result<(), PresentationError> {
    if let Some(key) = values.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(PresentationError::Invalid(format!(
            "theme has unsupported {family} key: {key}"
        )));
    }
    if values
        .values()
        .any(|value| value.len() > 128 || value.chars().any(char::is_control))
    {
        return Err(PresentationError::Invalid(format!(
            "theme {family} values must be control-free and at most 128 bytes"
        )));
    }
    Ok(())
}

fn legacy_style(
    values: &BTreeMap<String, String>,
    keys: &[&str],
    base: ThemeTextStyle,
) -> Result<ThemeTextStyle, PresentationError> {
    keys.iter()
        .find_map(|key| values.get(*key))
        .map_or(Ok(base), |value| parse_rich_style(value, base))
}

fn legacy_style_maps(
    maps: &[(&BTreeMap<String, String>, &[&str])],
    base: ThemeTextStyle,
) -> Result<ThemeTextStyle, PresentationError> {
    maps.iter()
        .find_map(|(values, keys)| keys.iter().find_map(|key| values.get(*key)))
        .map_or(Ok(base), |value| parse_rich_style(value, base))
}

fn rich_foreground(
    value: &str,
    base: Option<ThemeColor>,
) -> Result<Option<ThemeColor>, PresentationError> {
    parse_rich_style(
        value,
        ThemeTextStyle {
            foreground: base,
            bold: false,
            dim: false,
            italic: false,
        },
    )
    .map(|style| style.foreground)
}

fn parse_rich_style(
    value: &str,
    base: ThemeTextStyle,
) -> Result<ThemeTextStyle, PresentationError> {
    let mut style = ThemeTextStyle {
        foreground: base.foreground,
        bold: false,
        dim: false,
        italic: false,
    };
    let mut skip_background = false;
    for token in value.split_ascii_whitespace() {
        if skip_background {
            skip_background = false;
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "bold" => style.bold = true,
            "dim" => style.dim = true,
            "italic" => style.italic = true,
            "on" => skip_background = true,
            token if token.starts_with("bg:") => {}
            token => style.foreground = Some(parse_named_or_hex_color(token)?),
        }
    }
    Ok(style)
}

fn parse_named_or_hex_color(value: &str) -> Result<ThemeColor, PresentationError> {
    if value.starts_with('#') {
        return parse_color(value);
    }
    let (red, green, blue) = match value {
        "black" => (0, 0, 0),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "yellow" => (255, 255, 0),
        "blue" => (0, 0, 255),
        "magenta" => (255, 0, 255),
        "cyan" => (0, 255, 255),
        "white" => (255, 255, 255),
        "bright_black" => (128, 128, 128),
        "bright_red" => (255, 85, 85),
        "bright_green" => (85, 255, 85),
        "bright_yellow" => (255, 255, 85),
        "bright_blue" => (85, 85, 255),
        "bright_magenta" => (255, 85, 255),
        "bright_cyan" => (85, 255, 255),
        "bright_white" => (255, 255, 255),
        _ => {
            return Err(PresentationError::Invalid(format!(
                "unsupported theme color: {value}"
            )));
        }
    };
    Ok(ThemeColor { red, green, blue })
}

fn parse_spinner(value: &str) -> Result<ThemeSpinner, PresentationError> {
    match value {
        "dots" => Ok(ThemeSpinner::Dots),
        "line" => Ok(ThemeSpinner::Line),
        "arc" => Ok(ThemeSpinner::Arc),
        "bouncingBar" | "bouncing_bar" => Ok(ThemeSpinner::BouncingBar),
        "aesthetic" => Ok(ThemeSpinner::Aesthetic),
        _ => Err(PresentationError::Invalid(format!(
            "unsupported theme spinner: {value}"
        ))),
    }
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

fn validate_preferences(preferences: &ReplPreferences) -> Result<(), StoreError> {
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

/// Immutable-journal implementation of the presentation preference port.
pub struct EventSourcedPresentationRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedPresentationRepository {
    /// Bind the global REPL presentation profile to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }
}

impl PresentationRepository for EventSourcedPresentationRepository {
    fn load(&self) -> Result<ReplPreferences, StoreError> {
        let events = self.journal.read_stream(PREFERENCES_STREAM)?;
        let Some(event) = events.last() else {
            return Ok(ReplPreferences::default());
        };
        if event.event_type != PREFERENCES_UPDATED {
            return Err(StoreError::Verification(
                "presentation stream contains an unknown event".into(),
            ));
        }
        let payload = self.journal.decrypt_payload(event)?;
        let preferences: ReplPreferences = serde_json::from_value(
            payload
                .get("preferences")
                .cloned()
                .ok_or_else(|| StoreError::Verification("preferences payload is absent".into()))?,
        )
        .map_err(|error| StoreError::Verification(error.to_string()))?;
        validate_preferences(&preferences)?;
        Ok(preferences)
    }

    fn save(
        &self,
        preferences: ReplPreferences,
        actor: Actor,
    ) -> Result<ReplPreferences, StoreError> {
        validate_preferences(&preferences)?;
        let expected_stream_version =
            u64::try_from(self.journal.read_stream(PREFERENCES_STREAM)?.len())
                .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: PREFERENCES_STREAM.into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: PREFERENCES_UPDATED.into(),
            actor,
            context: ExecutionContext {
                correlation_id: PREFERENCES_STREAM.into(),
                ..ExecutionContext::default()
            },
            payload: json!({"preferences": &preferences}),
        })?;
        Ok(preferences)
    }

    fn list_history(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        if !(1..=MAX_HISTORY_ENTRIES).contains(&limit) {
            return Err(StoreError::Adapter(format!(
                "history limit must be between 1 and {MAX_HISTORY_ENTRIES}"
            )));
        }
        let events = self.journal.read_stream(HISTORY_STREAM)?;
        let skip = events.len().saturating_sub(limit);
        events
            .iter()
            .skip(skip)
            .map(|event| {
                if event.event_type != HISTORY_APPENDED {
                    return Err(StoreError::Verification(
                        "presentation history contains an unknown event".into(),
                    ));
                }
                let payload = self.journal.decrypt_payload(event)?;
                let entry = payload
                    .get("entry")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StoreError::Verification("presentation history entry is absent".into())
                    })?;
                validate_history_entry(entry)?;
                Ok(entry.into())
            })
            .collect()
    }

    fn append_history(&self, entry: String, actor: Actor) -> Result<String, StoreError> {
        validate_history_entry(&entry)?;
        let events = self.journal.read_stream(HISTORY_STREAM)?;
        if let Some(event) = events.last() {
            if event.event_type != HISTORY_APPENDED {
                return Err(StoreError::Verification(
                    "presentation history contains an unknown event".into(),
                ));
            }
            let payload = self.journal.decrypt_payload(event)?;
            if payload.get("entry").and_then(Value::as_str) == Some(entry.as_str()) {
                return Ok(entry);
            }
        }
        let expected_stream_version =
            u64::try_from(events.len()).map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: HISTORY_STREAM.into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: HISTORY_APPENDED.into(),
            actor,
            context: ExecutionContext {
                correlation_id: HISTORY_STREAM.into(),
                ..ExecutionContext::default()
            },
            payload: json!({"entry": &entry}),
        })?;
        Ok(entry)
    }
}

fn validate_history_entry(entry: &str) -> Result<(), StoreError> {
    if entry.trim().is_empty() || entry.len() > MAX_HISTORY_ENTRY_BYTES {
        return Err(StoreError::Adapter(format!(
            "history entry must be nonempty and at most {MAX_HISTORY_ENTRY_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Pure semantic renderer over already released contracts.
pub struct SemanticRenderer {
    preferences: ReplPreferences,
    color: bool,
}

impl SemanticRenderer {
    /// Create a renderer for one immutable preference snapshot.
    pub fn new(preferences: ReplPreferences) -> Self {
        Self {
            preferences,
            color: false,
        }
    }

    /// Enable or disable ANSI styling without changing semantic content.
    pub const fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    /// Apply the active assistant palette to released model text.
    pub fn assistant_text(&self, text: &str) -> String {
        TerminalPalette::for_preferences(&self.preferences)
            .assistant
            .paint(text, self.color)
    }

    fn label(&self, name: &str) -> String {
        let text = match self.preferences.theme {
            ThemeName::Default | ThemeName::Carrot | ThemeName::Hacker => format!("[{name}]"),
            ThemeName::Mono => format!("{name}:"),
            ThemeName::HighContrast => format!("{}:", name.to_ascii_uppercase()),
        };
        self.label_style(name).paint(&text, self.color)
    }

    fn label_style(&self, name: &str) -> TextStyle {
        let palette = TerminalPalette::for_preferences(&self.preferences);
        match name {
            "activity" => palette.activity,
            "thinking" => palette.thinking,
            "usage" => palette.meta,
            "approval" => palette.warning,
            "risk" | "error" => palette.error,
            "done" => palette.success,
            _ => palette.tool,
        }
    }

    /// Render current session work without exposing repository internals.
    pub fn work_state(&self, state: &WorkStateSnapshot) -> String {
        let summary = format!(
            "{} session={} tasks={}/{} decisions={} plans={} goals={} agents={}",
            self.label("work"),
            state.session_id,
            state.open_task_count,
            state.tasks.len(),
            state.active_decisions.len(),
            state.actionable_plans.len(),
            state.current_goals.len(),
            state.current_subagents.len()
        );
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            return summary;
        }
        let mut lines = vec![summary];
        lines.extend(
            state
                .tasks
                .iter()
                .filter(|task| {
                    !matches!(
                        task.status,
                        colossus_contracts::TaskStatus::Completed
                            | colossus_contracts::TaskStatus::Cancelled
                    )
                })
                .map(|task| format!("  task [{}] {}", task.id, task.title)),
        );
        lines.extend(
            state
                .current_goals
                .iter()
                .map(|goal| format!("  goal [{}] {}", goal.id, goal.objective)),
        );
        lines.join("\n")
    }

    /// Render context budget and compaction state.
    pub fn context_status(&self, status: &ContextStatus) -> String {
        format!(
            "{} session={} messages={} tokens={}/{} compacted={} snapshot={}",
            self.label("context"),
            status.session_id,
            status.message_count,
            status.token_estimate,
            status.context_window_tokens,
            status.compacted,
            status.active_snapshot_id.as_deref().unwrap_or("none")
        )
    }

    /// Render one already policy-released provider event.
    ///
    /// Visible model deltas are streamed separately and final output is not repeated. Safe
    /// reasoning summaries remain independently configurable from tool/activity events.
    pub fn provider_event(
        &self,
        event: &ProviderEvent,
    ) -> Result<Option<String>, PresentationError> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return Ok(None);
        }
        let rendered = match event {
            ProviderEvent::ModelDelta { .. } | ProviderEvent::FinalOutput { .. } => None,
            ProviderEvent::ReasoningSummary { summary } if self.preferences.show_reasoning => {
                Some(format!("{} {summary}", self.label("thinking")))
            }
            ProviderEvent::ReasoningSummary { .. } => None,
            ProviderEvent::ToolCallRequested { .. } => None,
            ProviderEvent::Usage { usage } => match self.preferences.events_mode {
                EventDisplayMode::Verbose => Some(format!(
                    "{} input={} output={} total={} cached={} reasoning={}",
                    self.label("usage"),
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.total_tokens,
                    usage
                        .cached_input_tokens
                        .map_or_else(|| "unknown".into(), |value| value.to_string()),
                    usage
                        .reasoning_tokens
                        .map_or_else(|| "unknown".into(), |value| value.to_string())
                )),
                EventDisplayMode::Compact | EventDisplayMode::Off => None,
            },
        };
        Ok(rendered)
    }

    /// Render one ordered application-level run event.
    pub fn run_event(&self, event: &RunEvent) -> Result<Option<String>, PresentationError> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return Ok(None);
        }
        match event {
            RunEvent::Provider { event } => self.provider_event(event),
            RunEvent::Phase {
                phase,
                turn,
                action,
                elapsed_seconds,
            } => Ok(self.render_phase(*phase, *turn, action.as_deref(), *elapsed_seconds)),
            RunEvent::ToolStarted {
                turn,
                call,
                elapsed_seconds,
            } => self.render_tool_started(*turn, call, *elapsed_seconds),
            RunEvent::ToolCompleted {
                turn,
                result,
                duration_seconds,
                elapsed_seconds,
            } => self.render_tool_completed(*turn, result, *duration_seconds, *elapsed_seconds),
            RunEvent::Error {
                code,
                message,
                recoverable,
                turn,
                elapsed_seconds,
            } => Ok(Some(self.with_detail(
                format!(
                    "{} code={} recoverable={} turn={} elapsed={:.2}s",
                    self.label("error"),
                    code,
                    if *recoverable { "yes" } else { "no" },
                    turn.map_or_else(|| "none".into(), |value| value.to_string()),
                    elapsed_seconds,
                ),
                Some(bounded_text(message, COMPACT_PREVIEW_CHARS)),
            ))),
        }
    }

    /// Render a correlated run event, including bounded provenance in verbose mode.
    pub fn run_event_envelope(
        &self,
        envelope: &RunEventEnvelope,
    ) -> Result<Option<String>, PresentationError> {
        let Some(rendered) = self.run_event(&envelope.event)? else {
            return Ok(None);
        };
        if self.preferences.events_mode == EventDisplayMode::Verbose {
            Ok(Some(format!(
                "run={} session={} {rendered}",
                envelope.run_id, envelope.session_id
            )))
        } else {
            Ok(Some(rendered))
        }
    }

    fn render_phase(
        &self,
        phase: RunPhase,
        turn: Option<u16>,
        action: Option<&str>,
        elapsed_seconds: f64,
    ) -> Option<String> {
        if phase == RunPhase::Completed && self.preferences.events_mode == EventDisplayMode::Off {
            return None;
        }
        let phase_name = match phase {
            RunPhase::Preparing => "preparing",
            RunPhase::WaitingForModel => "waiting_for_model",
            RunPhase::Responding => "responding",
            RunPhase::Completed => "completed",
        };
        match self.preferences.events_mode {
            EventDisplayMode::Verbose => Some(format!(
                "{} phase={phase_name} turn={} action={} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                turn.map_or_else(|| "none".into(), |value| value.to_string()),
                action.unwrap_or("none")
            )),
            EventDisplayMode::Compact | EventDisplayMode::Off => Some(format!(
                "{} {phase_name}{} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                action.map_or_else(String::new, |value| format!(" {value}"))
            )),
        }
    }

    fn render_tool_started(
        &self,
        turn: u16,
        call: &ToolCall,
        elapsed_seconds: f64,
    ) -> Result<Option<String>, PresentationError> {
        if self.preferences.events_mode == EventDisplayMode::Off {
            return Ok(Some(format!(
                "{} using {} elapsed={elapsed_seconds:.2}s",
                self.label("activity"),
                call.name
            )));
        }
        let family = ToolFamily::from_name(&call.name);
        let detail = summarize_value(&call.arguments, family.keys());
        let rendered = match self.preferences.events_mode {
            EventDisplayMode::Compact => self.with_detail(
                format!(
                    "{} start {} elapsed={elapsed_seconds:.2}s",
                    self.label(family.label()),
                    call.name,
                ),
                detail,
            ),
            EventDisplayMode::Verbose => format!(
                "{} start name={} call_id={} turn={} elapsed={elapsed_seconds:.2}s arguments={}",
                self.label(family.label()),
                call.name,
                call.call_id,
                turn,
                bounded_json(&call.arguments, VERBOSE_PREVIEW_CHARS)?
            ),
            EventDisplayMode::Off => unreachable!("handled above"),
        };
        Ok(Some(rendered))
    }

    fn render_tool_completed(
        &self,
        turn: u16,
        result: &ToolResult,
        duration_seconds: f64,
        elapsed_seconds: f64,
    ) -> Result<Option<String>, PresentationError> {
        let parsed = serde_json::from_str::<Value>(&result.output)
            .unwrap_or_else(|_| Value::String(result.output.clone()));
        let family = ToolFamily::from_name(&result.name);
        let recoverable = parsed
            .pointer("/error/recoverable")
            .and_then(Value::as_bool);
        let failed = result.exit_code != 0 || parsed.get("error").is_some();
        if self.preferences.events_mode == EventDisplayMode::Off && !failed {
            return Ok(None);
        }
        let status = if failed {
            if recoverable == Some(true) {
                "recoverable_error"
            } else {
                "failed"
            }
        } else {
            "ok"
        };
        let detail = if failed {
            parsed
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(|message| bounded_text(message, COMPACT_PREVIEW_CHARS))
        } else {
            summarize_value(&parsed, family.keys())
        };
        let rendered = match self.preferences.events_mode {
            EventDisplayMode::Verbose => format!(
                "{} complete name={} call_id={} turn={} status={} exit={} duration={duration_seconds:.2}s elapsed={elapsed_seconds:.2}s output={}",
                self.label(family.label()),
                result.name,
                result.call_id,
                turn,
                status,
                result.exit_code,
                bounded_json(&parsed, VERBOSE_PREVIEW_CHARS)?
            ),
            EventDisplayMode::Compact | EventDisplayMode::Off => self.with_detail(
                format!(
                    "{} complete {} status={} exit={} duration={duration_seconds:.2}s",
                    self.label(family.label()),
                    result.name,
                    status,
                    result.exit_code,
                ),
                detail,
            ),
        };
        Ok(Some(rendered))
    }

    fn with_detail(&self, summary: String, detail: Option<String>) -> String {
        let Some(detail) = detail else {
            return summary;
        };
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            format!("{summary} {detail}")
        } else {
            format!("{summary}\n  {detail}")
        }
    }

    /// Render generic released structured output according to transcript density.
    pub fn structured(&self, value: &Value) -> Result<String, PresentationError> {
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            serde_json::to_string(value)
        } else {
            serde_json::to_string_pretty(value)
        }
        .map_err(|error| PresentationError::Invalid(error.to_string()))
    }
}

#[derive(Clone, Copy)]
enum ToolFamily {
    Files,
    Shell,
    Git,
    Work,
    Context,
    Repository,
    Skills,
    Web,
    Mcp,
    Trace,
    Integrations,
    Packs,
    Generic,
}

impl ToolFamily {
    fn from_name(name: &str) -> Self {
        let prefix = name.split('.').next().unwrap_or(name);
        match prefix {
            "filesystem" | "patch" => Self::Files,
            "shell" | "process" => Self::Shell,
            "git" => Self::Git,
            "task" | "decision" | "plan" | "goal" | "agent" | "memory" => Self::Work,
            "context" => Self::Context,
            "repo" => Self::Repository,
            "skill" => Self::Skills,
            "web" | "docs" | "network" => Self::Web,
            "mcp" => Self::Mcp,
            "trace" | "telemetry" | "audit" => Self::Trace,
            "integration" => Self::Integrations,
            "pack" | "bundle" => Self::Packs,
            _ => Self::Generic,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Files => "file",
            Self::Shell => "shell",
            Self::Git => "git",
            Self::Work => "work",
            Self::Context => "context",
            Self::Repository => "repo",
            Self::Skills => "skill",
            Self::Web => "web",
            Self::Mcp => "mcp",
            Self::Trace => "trace",
            Self::Integrations => "integration",
            Self::Packs => "pack",
            Self::Generic => "tool",
        }
    }

    const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Files => &[
                "path",
                "bytes",
                "matches",
                "changed",
                "line_start",
                "line_end",
            ],
            Self::Shell => &["executable", "exit_code", "stdout", "stderr", "truncated"],
            Self::Git => &["branch", "commit", "path", "status", "summary", "stdout"],
            Self::Work => &["id", "status", "title", "objective", "open_task_count"],
            Self::Context => &[
                "session_id",
                "message_count",
                "token_estimate",
                "snapshot_id",
                "compacted",
            ],
            Self::Repository => &["path", "symbol", "matches", "files", "summary"],
            Self::Skills => &["name", "path", "status", "sha256"],
            Self::Web => &["url", "status", "title", "media_type", "bytes"],
            Self::Mcp => &["server", "tool", "status", "content"],
            Self::Trace => &["run_id", "event_count", "path", "status"],
            Self::Integrations => &["name", "tool", "status", "connected"],
            Self::Packs => &["name", "version", "trusted", "status", "publisher"],
            Self::Generic => &["id", "name", "status", "message"],
        }
    }
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    let mut rendered = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        rendered.push('…');
    }
    rendered.replace(['\n', '\r'], " ")
}

fn bounded_json(value: &Value, max_chars: usize) -> Result<String, PresentationError> {
    serde_json::to_string(value)
        .map(|encoded| bounded_text(&encoded, max_chars))
        .map_err(|error| PresentationError::Invalid(error.to_string()))
}

fn summarize_value(value: &Value, keys: &[&str]) -> Option<String> {
    let parts = keys
        .iter()
        .filter_map(|key| {
            find_key(value, key, 0).map(|value| {
                format!(
                    "{key}={}",
                    bounded_text(&scalar_summary(value), COMPACT_PREVIEW_CHARS / 2)
                )
            })
        })
        .take(4)
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn find_key<'a>(value: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 2 {
        return None;
    }
    let object = value.as_object()?;
    if let Some(value) = object.get(key) {
        return Some(value);
    }
    object
        .values()
        .find_map(|value| find_key(value, key, depth.saturating_add(1)))
}

fn scalar_summary(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => format!("{} items", values.len()),
        Value::Object(values) => format!("{} fields", values.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EventDisplayMode, EventSourcedPresentationRepository, MAX_CUSTOM_THEMES,
        MAX_THEME_FILE_BYTES, ReplPreferences, SemanticRenderer, StreamDisplayMode,
        TerminalPalette, ThemeLibrary, ThemeName, TranscriptDensity,
    };
    use colossus_contracts::{
        Actor, ActorType, ProviderEvent, ProviderUsage, RunEvent, RunEventEnvelope, RunPhase,
        ToolCall, ToolResult, WorkStateSnapshot,
    };
    use colossus_ports::{EventJournal, PresentationRepository};
    use colossus_testkit::{InMemoryEventJournal, assert_presentation_repository_conformance};
    use std::{fs, sync::Arc};
    use tempfile::tempdir;

    #[test]
    fn preferences_reconstruct_from_immutable_events_and_validate_schema() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        assert_eq!(
            repository.load().expect("defaults"),
            ReplPreferences::default()
        );
        let preferences = ReplPreferences {
            theme: ThemeName::HighContrast,
            multiline: true,
            stream_mode: StreamDisplayMode::Off,
            events_mode: EventDisplayMode::Verbose,
            show_reasoning: false,
            transcript_density: TranscriptDensity::Compact,
            ..ReplPreferences::default()
        };
        repository
            .save(
                preferences.clone(),
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
            )
            .expect("save");
        let restarted = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        assert_eq!(restarted.load().expect("load"), preferences);
        let events = journal.read_stream("presentation:repl").expect("events");
        assert_eq!(events[0].event_type, "presentation.preferences.updated.v1");
        let invalid = ReplPreferences {
            schema_version: 2,
            ..ReplPreferences::default()
        };
        assert!(
            restarted
                .save(
                    invalid,
                    Actor {
                        actor_type: ActorType::User,
                        id: "terminal-user".into(),
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn event_sourced_repository_passes_shared_conformance() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(journal);
        assert_presentation_repository_conformance(&repository);
    }

    #[test]
    fn work_renderer_has_compact_and_comfortable_semantics() {
        let state = WorkStateSnapshot {
            session_id: "session-1".into(),
            tasks: Vec::new(),
            open_task_count: 0,
            active_decisions: Vec::new(),
            actionable_plans: Vec::new(),
            current_goals: Vec::new(),
            current_subagents: Vec::new(),
        };
        let compact = SemanticRenderer::new(ReplPreferences {
            transcript_density: TranscriptDensity::Compact,
            ..ReplPreferences::default()
        });
        assert_eq!(
            compact.work_state(&state),
            "[work] session=session-1 tasks=0/0 decisions=0 plans=0 goals=0 agents=0"
        );
        let comfortable = SemanticRenderer::new(ReplPreferences::default());
        assert!(
            comfortable
                .work_state(&state)
                .starts_with("[work] session=session-1")
        );
    }

    #[test]
    fn provider_events_respect_reasoning_events_and_theme_independently() {
        let renderer = SemanticRenderer::new(ReplPreferences {
            theme: ThemeName::HighContrast,
            events_mode: EventDisplayMode::Off,
            show_reasoning: true,
            ..ReplPreferences::default()
        });
        assert_eq!(
            renderer
                .provider_event(&ProviderEvent::ReasoningSummary {
                    summary: "safe summary".into(),
                })
                .expect("reasoning"),
            Some("THINKING: safe summary".into())
        );
        assert_eq!(
            renderer
                .provider_event(&ProviderEvent::ToolCallRequested {
                    call_id: "call-1".into(),
                    name: "filesystem.read".into(),
                    arguments: serde_json::json!({"path": "README.md"}),
                })
                .expect("tool"),
            None
        );

        let verbose = SemanticRenderer::new(ReplPreferences {
            theme: ThemeName::Mono,
            events_mode: EventDisplayMode::Verbose,
            ..ReplPreferences::default()
        });
        assert_eq!(
            verbose
                .provider_event(&ProviderEvent::Usage {
                    usage: ProviderUsage {
                        input_tokens: 4,
                        output_tokens: 2,
                        total_tokens: 6,
                        cached_input_tokens: Some(1),
                        reasoning_tokens: None,
                    },
                })
                .expect("usage"),
            Some("usage: input=4 output=2 total=6 cached=1 reasoning=unknown".into())
        );
        let correlated = verbose
            .run_event_envelope(&RunEventEnvelope {
                schema_version: 1,
                run_id: "run-1".into(),
                session_id: "session-1".into(),
                event: RunEvent::Phase {
                    phase: RunPhase::Preparing,
                    turn: Some(1),
                    action: None,
                    elapsed_seconds: 0.1,
                },
            })
            .expect("correlated")
            .expect("visible");
        assert!(correlated.starts_with("run=run-1 session=session-1"));
    }

    #[test]
    fn semantic_tool_families_errors_and_elapsed_phases_are_distinct() {
        let renderer = SemanticRenderer::new(ReplPreferences::default());
        for (name, label) in [
            ("filesystem.read", "[file]"),
            ("shell.run", "[shell]"),
            ("git.status", "[git]"),
            ("task.list", "[work]"),
            ("context.show", "[context]"),
            ("repo.map", "[repo]"),
            ("skill.read", "[skill]"),
            ("web.fetch", "[web]"),
            ("mcp.call", "[mcp]"),
            ("trace.export", "[trace]"),
            ("integration.invoke", "[integration]"),
            ("pack.verify", "[pack]"),
            ("echo", "[tool]"),
        ] {
            let rendered = renderer
                .run_event(&RunEvent::ToolStarted {
                    turn: 1,
                    call: ToolCall {
                        call_id: format!("call-{name}"),
                        name: name.into(),
                        arguments: serde_json::json!({"path": "README.md", "name": "demo"}),
                    },
                    elapsed_seconds: 0.25,
                })
                .expect("render")
                .expect("visible");
            assert!(rendered.starts_with(label), "{name}: {rendered}");
        }

        let completed = renderer
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-file".into(),
                    name: "filesystem.read".into(),
                    output: serde_json::json!({"path": "README.md", "bytes": 42}).to_string(),
                    exit_code: 0,
                },
                duration_seconds: 1.25,
                elapsed_seconds: 2.0,
            })
            .expect("render")
            .expect("visible");
        assert!(completed.contains("duration=1.25s"));
        assert!(completed.contains("path=README.md"));

        let quiet = SemanticRenderer::new(ReplPreferences {
            events_mode: EventDisplayMode::Off,
            ..ReplPreferences::default()
        });
        assert!(
            quiet
                .run_event(&RunEvent::ToolCompleted {
                    turn: 1,
                    result: ToolResult {
                        call_id: "call-ok".into(),
                        name: "echo".into(),
                        output: "ok".into(),
                        exit_code: 0,
                    },
                    duration_seconds: 0.1,
                    elapsed_seconds: 0.2,
                })
                .expect("quiet")
                .is_none()
        );
        let recoverable = quiet
            .run_event(&RunEvent::ToolCompleted {
                turn: 1,
                result: ToolResult {
                    call_id: "call-error".into(),
                    name: "filesystem.read".into(),
                    output: serde_json::json!({
                        "error": {"message": "missing", "recoverable": true}
                    })
                    .to_string(),
                    exit_code: 1,
                },
                duration_seconds: 0.1,
                elapsed_seconds: 0.2,
            })
            .expect("error")
            .expect("visible error");
        assert!(recoverable.contains("status=recoverable_error"));
        let phase = quiet
            .run_event(&RunEvent::Phase {
                phase: RunPhase::WaitingForModel,
                turn: Some(2),
                action: Some("model-x".into()),
                elapsed_seconds: 3.5,
            })
            .expect("phase")
            .expect("activity remains visible");
        assert!(phase.contains("waiting_for_model model-x elapsed=3.50s"));
    }

    #[test]
    fn every_builtin_palette_styles_tty_output_without_touching_redirected_text() {
        for theme in [
            ThemeName::Default,
            ThemeName::Mono,
            ThemeName::HighContrast,
            ThemeName::Carrot,
            ThemeName::Hacker,
        ] {
            let preferences = ReplPreferences {
                theme,
                ..ReplPreferences::default()
            };
            let event = RunEvent::Phase {
                phase: RunPhase::Preparing,
                turn: Some(1),
                action: None,
                elapsed_seconds: 0.5,
            };
            let redirected = SemanticRenderer::new(preferences.clone())
                .run_event(&event)
                .expect("redirected render")
                .expect("visible");
            assert!(!redirected.contains("\x1b["), "{}", theme.as_str());
            assert!(
                !SemanticRenderer::new(preferences.clone())
                    .assistant_text("connected")
                    .contains("\x1b[")
            );
            let terminal = SemanticRenderer::new(preferences)
                .with_color(true)
                .run_event(&event)
                .expect("terminal render")
                .expect("visible");
            assert!(terminal.contains("\x1b["), "{}", theme.as_str());
            let palette = TerminalPalette::for_theme(theme);
            assert_ne!(
                palette.activity_frame(0.0, false),
                palette.activity_frame(0.1, false),
                "{}",
                theme.as_str()
            );
        }
        let assistant = SemanticRenderer::new(ReplPreferences {
            theme: ThemeName::Hacker,
            ..ReplPreferences::default()
        })
        .with_color(true)
        .assistant_text("connected");
        assert!(assistant.contains("\x1b["));
        assert!(assistant.contains("connected"));
    }

    #[test]
    fn bounded_json_and_toml_theme_files_resolve_into_hash_bound_snapshots() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        fs::write(
            themes.join("ocean.json"),
            r##"{
              "schemaVersion": 1,
              "name": "ocean",
              "base": "default",
              "title": "Ocean",
              "caret": ">",
              "continuation": "|",
              "prompt": {
                "left": "#00ffff",
                "indicator": "#00d7ff"
              },
              "styles": {
                "assistant": {"foreground": "#d7ffff"},
                "tool": {"foreground": "#00afff", "bold": true}
              },
              "spinner": "line"
            }"##,
        )
        .expect("ocean");
        fs::write(
            themes.join("ember.toml"),
            r##"schemaVersion = 1
name = "ember"
base = "carrot"
spinner = "aesthetic"

[prompt]
right = "#ffaf5f"

[styles.warning]
foreground = "#ffff00"
bold = true
"##,
        )
        .expect("ember");

        let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
        assert_eq!(
            library.names(),
            vec![
                "default",
                "mono",
                "high_contrast",
                "carrot",
                "hacker",
                "ember",
                "ocean",
            ]
        );
        let mut preferences = ReplPreferences::default();
        library
            .select("OCEAN", &mut preferences)
            .expect("select ocean");
        assert_eq!(preferences.theme_name(), "ocean");
        let ocean = preferences.custom_theme.as_ref().expect("snapshot");
        assert_eq!(ocean.source_hash.len(), 64);
        assert_eq!(ocean.prompt_left.expect("left").green, 255);
        assert_eq!(ocean.spinner, colossus_contracts::ThemeSpinner::Line);
        let palette = TerminalPalette::for_preferences(&preferences);
        assert_ne!(
            palette.activity_frame(0.0, false),
            palette.activity_frame(0.1, false)
        );
        let terminal = SemanticRenderer::new(preferences.clone())
            .with_color(true)
            .assistant_text("connected");
        assert!(terminal.contains("38;2;215;255;255"));
        assert!(terminal.contains("connected"));
        assert!(
            !SemanticRenderer::new(preferences)
                .assistant_text("connected")
                .contains("\x1b[")
        );

        let preview = library.preview("ember").expect("preview ember");
        assert_eq!(preview.base, ThemeName::Carrot);
        assert_eq!(preview.spinner, colossus_contracts::ThemeSpinner::Aesthetic);
    }

    #[test]
    fn legacy_python_theme_schema_is_strictly_mapped_during_cutover() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        fs::write(
            themes.join("ocean.json"),
            r##"{
              "name": "ocean",
              "title": "Ocean",
              "caret": ">",
              "continuation": "|",
              "styles": {
                "prompt.title": "#00ffff bold",
                "prompt.caret": "bright_cyan"
              },
              "trace": {"tool_call": "bold cyan"},
              "transcript": {
                "assistant": "#d7ffff",
                "tool": "bold blue",
                "activity_spinner": "line"
              }
            }"##,
        )
        .expect("legacy theme");

        let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
        let ocean = library.preview("ocean").expect("legacy preview");
        assert_eq!(ocean.base, ThemeName::Default);
        assert_eq!(ocean.title, "Ocean");
        assert_eq!(ocean.prompt_left.expect("prompt color").green, 255);
        assert_eq!(ocean.indicator.expect("indicator").blue, 255);
        assert_eq!(ocean.assistant.foreground.expect("assistant").red, 215);
        assert!(ocean.tool.bold);
        assert_eq!(ocean.tool.foreground.expect("tool").green, 255);
        assert_eq!(ocean.spinner, colossus_contracts::ThemeSpinner::Line);

        fs::write(
            themes.join("invalid.json"),
            r#"{"name":"invalid","transcript":{"activity_spinner":"unknown"}}"#,
        )
        .expect("invalid legacy theme");
        assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
    }

    #[test]
    fn custom_theme_snapshot_reconstructs_without_rereading_mutated_source() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        let source = themes.join("stable.json");
        fs::write(
            &source,
            r##"{"schemaVersion":1,"name":"stable","styles":{"assistant":{"foreground":"#010203"}}}"##,
        )
        .expect("theme");
        let library = ThemeLibrary::load(std::slice::from_ref(&themes)).expect("library");
        let mut preferences = ReplPreferences::default();
        library.select("stable", &mut preferences).expect("select");
        let selected_hash = preferences
            .custom_theme
            .as_ref()
            .expect("custom")
            .source_hash
            .clone();
        fs::write(
            source,
            r##"{"schemaVersion":1,"name":"stable","styles":{"assistant":{"foreground":"#ffffff"}}}"##,
        )
        .expect("mutate source");

        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        repository
            .save(
                preferences.clone(),
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
            )
            .expect("save snapshot");
        let loaded = EventSourcedPresentationRepository::new(journal)
            .load()
            .expect("load snapshot");
        assert_eq!(loaded, preferences);
        assert_eq!(
            loaded.custom_theme.as_ref().expect("custom").source_hash,
            selected_hash
        );
        assert_eq!(
            loaded
                .custom_theme
                .as_ref()
                .expect("custom")
                .assistant
                .foreground
                .expect("color")
                .red,
            1
        );
    }

    #[test]
    fn theme_library_rejects_unknown_fields_collisions_and_symlinks() {
        let directory = tempdir().expect("directory");
        let themes = directory.path().join("themes");
        fs::create_dir(&themes).expect("themes");
        fs::write(
            themes.join("invalid.json"),
            r#"{"schemaVersion":1,"name":"invalid","executable":"no"}"#,
        )
        .expect("invalid");
        assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
        fs::remove_file(themes.join("invalid.json")).expect("remove invalid");
        fs::write(
            themes.join("builtin.toml"),
            "schemaVersion = 1\nname = \"hacker\"\n",
        )
        .expect("builtin");
        assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());

        #[cfg(unix)]
        {
            fs::remove_file(themes.join("builtin.toml")).expect("remove builtin");
            let outside = directory.path().join("outside.json");
            fs::write(&outside, r#"{"schemaVersion":1,"name":"outside"}"#).expect("outside");
            std::os::unix::fs::symlink(&outside, themes.join("linked.json")).expect("symlink");
            assert!(ThemeLibrary::load(std::slice::from_ref(&themes)).is_err());
        }
    }

    #[test]
    fn theme_library_enforces_file_size_count_and_color_bounds() {
        let directory = tempdir().expect("directory");

        let oversized = directory.path().join("oversized");
        fs::create_dir(&oversized).expect("oversized directory");
        fs::write(
            oversized.join("large.json"),
            vec![b' '; MAX_THEME_FILE_BYTES as usize + 1],
        )
        .expect("oversized theme");
        assert!(ThemeLibrary::load(std::slice::from_ref(&oversized)).is_err());

        let invalid_color = directory.path().join("invalid-color");
        fs::create_dir(&invalid_color).expect("invalid color directory");
        fs::write(
            invalid_color.join("invalid.json"),
            r##"{"schemaVersion":1,"name":"invalid","prompt":{"left":"red"}}"##,
        )
        .expect("invalid color");
        assert!(ThemeLibrary::load(std::slice::from_ref(&invalid_color)).is_err());

        let excess = directory.path().join("excess");
        fs::create_dir(&excess).expect("excess directory");
        for index in 0..=MAX_CUSTOM_THEMES {
            fs::write(
                excess.join(format!("theme-{index:02}.json")),
                format!(r#"{{"schemaVersion":1,"name":"theme_{index:02}"}}"#),
            )
            .expect("theme");
        }
        assert!(ThemeLibrary::load(std::slice::from_ref(&excess)).is_err());

        #[cfg(unix)]
        {
            let real = directory.path().join("real");
            fs::create_dir(&real).expect("real directory");
            let linked = directory.path().join("linked");
            std::os::unix::fs::symlink(real, &linked).expect("directory symlink");
            assert!(ThemeLibrary::load(&[linked]).is_err());
        }
    }
}
