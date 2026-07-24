//! Event-sourced presentation preferences and pure semantic terminal rendering.

use colossus_contracts::{
    Actor, AutomaticApprovalNotice, ContextStatus, CustomTheme, EventClassification,
    ExecutionContext, NewEvent, ProviderEvent, RiskLevel, RiskReviewFailure,
    RiskReviewFallbackNotice, RunEvent, RunEventEnvelope, RunPhase, ThemeColor, ThemeSpinner,
    ThemeTextStyle, ToolCall, ToolResult, WorkStateSnapshot,
};
pub use colossus_contracts::{
    EventDisplayMode, StreamDisplayMode, TerminalPreferences, ThemeName, TranscriptDensity,
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// Persisted compatibility identifier; changing it would orphan existing preferences.
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

mod document;
mod palette;
mod repository;
mod semantic;
mod terminal;
mod themes;

pub use document::{
    PresentationBlock, PresentationDocument, PresentationError, PresentationTable,
    PresentationTone, StyledLine, StyledSpan, document_from_json,
};
pub use palette::{RgbColor, TerminalPalette};
pub use repository::EventSourcedPresentationRepository;
pub use semantic::{
    SemanticRenderer, automatic_approval_document, context_status_document,
    risk_review_fallback_document, tool_result_document, work_state_document,
};
pub use terminal::{StyledDocumentRenderer, TerminalDocumentRenderer};
pub use themes::{ThemeLibrary, ThemeLibraryStatus, ThemeScaffold, default_user_theme_directory};

use document::{human_field_name, json_block};
use palette::TextStyle;
use semantic::bounded_text;
#[cfg(test)]
use terminal::display_width;
use themes::validate_preferences;

#[cfg(test)]
mod tests;
