use super::*;

/// Terminal RGB value shared by interactive prompts and semantic transcript palettes.
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
pub(super) struct TextStyle {
    foreground: Option<RgbColor>,
    bold: bool,
    dim: bool,
    italic: bool,
}

impl TextStyle {
    pub(super) const fn color(foreground: RgbColor) -> Self {
        Self {
            foreground: Some(foreground),
            bold: false,
            dim: false,
            italic: false,
        }
    }

    pub(super) const fn plain() -> Self {
        Self {
            foreground: None,
            bold: false,
            dim: false,
            italic: false,
        }
    }

    pub(super) const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub(super) const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    pub(super) const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub(super) fn paint(self, text: &str, enabled: bool) -> String {
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
    pub(super) prompt_left: Option<RgbColor>,
    pub(super) prompt_right: Option<RgbColor>,
    pub(super) indicator: Option<RgbColor>,
    pub(super) continuation: Option<RgbColor>,
    pub(super) assistant: TextStyle,
    pub(super) activity: TextStyle,
    pub(super) thinking: TextStyle,
    pub(super) tool: TextStyle,
    pub(super) success: TextStyle,
    pub(super) warning: TextStyle,
    pub(super) error: TextStyle,
    pub(super) meta: TextStyle,
    pub(super) spinner_frames: &'static [&'static str],
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
                meta: TextStyle::color(RgbColor::new(174, 184, 194)),
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
                meta: TextStyle::color(RgbColor::new(220, 174, 136)),
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
                meta: TextStyle::color(RgbColor::new(112, 222, 146)),
                spinner_frames: &["░", "▒", "▓", "█", "▓", "▒"],
            },
        }
    }

    /// Resolve a built-in palette or an immutable custom snapshot.
    pub fn for_preferences(preferences: &TerminalPreferences) -> Self {
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

    /// Metadata style used for low-emphasis terminal affordances such as type-ahead.
    pub fn meta_style(self) -> ThemeTextStyle {
        self.meta.into()
    }

    /// Assistant transcript style.
    pub fn assistant_style(self) -> ThemeTextStyle {
        self.assistant.into()
    }

    /// Operator/user transcript style derived from the prompt identity color.
    pub fn user_style(self) -> ThemeTextStyle {
        let mut style: ThemeTextStyle = self.meta.into();
        style.foreground = self.prompt_left.map(Into::into);
        style.bold = true;
        style.dim = false;
        style
    }

    /// Neutral semantic-section accent used to break up transcript hierarchy.
    pub fn section_style(self) -> ThemeTextStyle {
        let mut style = self.user_style();
        style.italic = false;
        style
    }

    /// Active background-operation style.
    pub fn activity_style(self) -> ThemeTextStyle {
        self.activity.into()
    }

    /// Tool and integration transcript style.
    pub fn tool_style(self) -> ThemeTextStyle {
        self.tool.into()
    }

    /// Warning and attention style.
    pub fn warning_style(self) -> ThemeTextStyle {
        self.warning.into()
    }

    /// Error and denial style.
    pub fn error_style(self) -> ThemeTextStyle {
        self.error.into()
    }

    /// Theme-resolved style for one semantic presentation tone.
    pub fn tone_style(self, tone: PresentationTone) -> ThemeTextStyle {
        match tone {
            PresentationTone::Neutral => self.assistant,
            PresentationTone::Success => self.success,
            PresentationTone::Warning => self.warning,
            PresentationTone::Error => self.error,
            PresentationTone::Thinking => self.thinking,
            PresentationTone::Tool => self.tool,
        }
        .into()
    }

    pub(super) fn style_for_block(self, block: &PresentationBlock) -> ThemeTextStyle {
        match block {
            PresentationBlock::Card { tone, .. } => return self.tone_style(*tone),
            PresentationBlock::Table(_)
            | PresentationBlock::KeyValue(_)
            | PresentationBlock::Prompt { .. } => self.meta,
            PresentationBlock::Code { .. } | PresentationBlock::Diff(_) => self.tool,
            PresentationBlock::Text(_) | PresentationBlock::Markdown(_) => self.assistant,
            PresentationBlock::Blank => TextStyle::plain(),
        }
        .into()
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
