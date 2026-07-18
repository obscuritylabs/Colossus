use super::*;

/// Built-in terminal theme identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    /// Balanced terminal labels.
    #[default]
    Default,
    /// Color-free terminal rendering.
    #[serde(alias = "plain")]
    Mono,
    /// Strong uppercase labels without relying on color perception.
    HighContrast,
    /// Warm orange terminal palette.
    Carrot,
    /// Green-on-dark terminal palette.
    Hacker,
}

impl ThemeName {
    /// Stable configuration and command spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Mono => "mono",
            Self::HighContrast => "high_contrast",
            Self::Carrot => "carrot",
            Self::Hacker => "hacker",
        }
    }

    /// Resolve a stable built-in spelling, including compatibility aliases.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "default" => Some(Self::Default),
            "mono" | "plain" => Some(Self::Mono),
            "high_contrast" => Some(Self::HighContrast),
            "carrot" => Some(Self::Carrot),
            "hacker" => Some(Self::Hacker),
            _ => None,
        }
    }
}

/// Serializable RGB color used by immutable custom-theme snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeColor {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

/// Serializable text emphasis used by immutable custom-theme snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeTextStyle {
    /// Optional terminal foreground color.
    pub foreground: Option<ThemeColor>,
    /// Bold emphasis.
    pub bold: bool,
    /// Dim emphasis.
    pub dim: bool,
    /// Italic emphasis.
    pub italic: bool,
}

/// Bounded built-in animation selected by a custom theme.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeSpinner {
    /// Braille dot rotation.
    #[default]
    Dots,
    /// ASCII line rotation.
    Line,
    /// Circular arc rotation.
    Arc,
    /// Horizontal fill animation.
    #[serde(alias = "bouncing_bar")]
    BouncingBar,
    /// Shaded block animation.
    Aesthetic,
}

/// Fully resolved, hash-bound, data-only custom terminal theme.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomTheme {
    /// Custom theme contract version.
    pub schema_version: u16,
    /// Simple custom identity.
    pub name: String,
    /// SHA-256 of the source file bytes.
    pub source_hash: String,
    /// Built-in label behavior inherited by the custom theme.
    pub base: ThemeName,
    /// Prompt title.
    pub title: String,
    /// Single-line prompt indicator.
    pub caret: String,
    /// Multiline prompt indicator.
    pub continuation: String,
    /// Left-prompt color.
    pub prompt_left: Option<ThemeColor>,
    /// Right-prompt color.
    pub prompt_right: Option<ThemeColor>,
    /// Prompt-indicator color.
    pub indicator: Option<ThemeColor>,
    /// Continuation-indicator color.
    pub continuation_color: Option<ThemeColor>,
    /// Assistant text style.
    pub assistant: ThemeTextStyle,
    /// Activity text style.
    pub activity: ThemeTextStyle,
    /// Safe reasoning-summary style.
    pub thinking: ThemeTextStyle,
    /// Tool label/result style.
    pub tool: ThemeTextStyle,
    /// Successful result style.
    pub success: ThemeTextStyle,
    /// Approval/warning style.
    pub warning: ThemeTextStyle,
    /// Risk/error style.
    pub error: ThemeTextStyle,
    /// Metadata style.
    pub meta: ThemeTextStyle,
    /// Bounded activity animation.
    pub spinner: ThemeSpinner,
}

/// Provider/activity event detail rendered by terminal interfaces.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventDisplayMode {
    /// One bounded semantic line per meaningful activity.
    #[default]
    Compact,
    /// Full released structured event content.
    Verbose,
    /// Suppress activity events while preserving final output and errors.
    Off,
}

impl EventDisplayMode {
    /// Stable configuration and command spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Verbose => "verbose",
            Self::Off => "off",
        }
    }
}

/// Model-output streaming behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamDisplayMode {
    /// Stream visible text alongside configured semantic events.
    #[default]
    On,
    /// Stream only normalized visible text, suppressing semantic event blocks.
    Raw,
    /// Buffer visible model output until the run finishes.
    Off,
}

impl StreamDisplayMode {
    /// Stable configuration and command spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Raw => "raw",
            Self::Off => "off",
        }
    }
}

/// Transcript vertical-density preference.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptDensity {
    /// Readable blocks with labels and spacing.
    #[default]
    Comfortable,
    /// Minimal vertical space.
    Compact,
}

impl TranscriptDensity {
    /// Stable configuration and command spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Compact => "compact",
        }
    }
}

/// Strict versioned interactive-terminal presentation preferences.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalPreferences {
    /// Preference schema version.
    pub schema_version: u16,
    /// Built-in theme identity, or the inheritance base for a custom snapshot.
    pub theme: ThemeName,
    /// Immutable custom theme snapshot, when one is selected.
    #[serde(default)]
    pub custom_theme: Option<CustomTheme>,
    /// Whether the editor composes multiple lines before submission.
    pub multiline: bool,
    /// How released model output streams.
    pub stream_mode: StreamDisplayMode,
    /// Activity event detail.
    pub events_mode: EventDisplayMode,
    /// Whether safe reasoning summaries are visible.
    pub show_reasoning: bool,
    /// Transcript vertical density.
    pub transcript_density: TranscriptDensity,
}

impl Default for TerminalPreferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: ThemeName::Default,
            custom_theme: None,
            multiline: false,
            stream_mode: StreamDisplayMode::On,
            events_mode: EventDisplayMode::Compact,
            show_reasoning: true,
            transcript_density: TranscriptDensity::Comfortable,
        }
    }
}

impl TerminalPreferences {
    /// Effective built-in or custom theme identity.
    pub fn theme_name(&self) -> &str {
        self.custom_theme
            .as_ref()
            .map_or_else(|| self.theme.as_str(), |theme| theme.name.as_str())
    }

    /// Select a built-in theme and clear any prior custom snapshot.
    pub fn select_builtin_theme(&mut self, theme: ThemeName) {
        self.theme = theme;
        self.custom_theme = None;
    }

    /// Select an immutable custom theme snapshot.
    pub fn select_custom_theme(&mut self, theme: CustomTheme) {
        self.theme = theme.base;
        self.custom_theme = Some(theme);
    }
}
