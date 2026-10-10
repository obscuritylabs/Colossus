use super::{BrowserDocumentId, BrowserElementId, BrowserSnapshotId, BrowserTabId, BrowserUrl};
use serde::{Deserialize, Serialize};

/// Current document selected by a trusted session mapping, never a native CDP target.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserTarget {
    /// Owned opaque tab.
    pub tab_id: BrowserTabId,
    /// Exact current document identity.
    pub document_id: BrowserDocumentId,
}

/// A token is usable only with the exact document and latest accepted snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserElementRef {
    /// Document owning this element.
    pub document_id: BrowserDocumentId,
    /// Snapshot that issued this token.
    pub snapshot_id: BrowserSnapshotId,
    /// Opaque element identity, resolved privately by the engine adapter.
    pub element_id: BrowserElementId,
}

/// Safe guest keys. Application accelerators and arbitrary key sequences are absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserKey {
    /// Submit or activate the focused control.
    Enter,
    /// Advance guest focus.
    Tab,
    /// Dismiss a guest interaction.
    Escape,
    /// Remove the preceding character.
    Backspace,
    /// Remove the following character.
    Delete,
    /// Move upward.
    ArrowUp,
    /// Move downward.
    ArrowDown,
    /// Move left.
    ArrowLeft,
    /// Move right.
    ArrowRight,
    /// Move to the start.
    Home,
    /// Move to the end.
    End,
    /// Move one page upward.
    PageUp,
    /// Move one page downward.
    PageDown,
    /// Activate with a space.
    Space,
}

/// Bounded wait predicates, with no arbitrary script or selector language.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserWaitCondition {
    /// Wait for the current document's load completion.
    Load {},
    /// Wait for one fresh referenced element to become visible.
    ElementVisible {
        /// Exact snapshot reference.
        element: BrowserElementRef,
    },
}

/// Exact engine operations used for capability checks before dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserActionKind {
    /// Navigate to a permitted HTTP(S) destination.
    Navigate,
    /// Read a semantic snapshot.
    Snapshot,
    /// Capture a bounded PNG for mandatory post-effect artifact release.
    Screenshot,
    /// Upload a pre-authorized owner-bound artifact into a current ordinary file input.
    Upload,
    /// Download one current linked resource into private custody before release.
    Download,
    /// Activate an element.
    Click,
    /// Fill an ordinary noncredential field.
    Fill,
    /// Select bounded options.
    Select,
    /// Press a safe guest key.
    Press,
    /// Scroll a bounded distance.
    Scroll,
    /// Wait for a bounded predicate.
    Wait,
    /// Navigate back.
    Back,
    /// Navigate forward.
    Forward,
    /// Reload the current page.
    Reload,
    /// Stop a pending navigation.
    Stop,
    /// Allocate one tab.
    TabOpen,
    /// Select an owned tab.
    TabSelect,
    /// Close an owned tab.
    TabClose,
}

/// Typed browser operations. Every operation still requires its runtime effect permit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserAction {
    /// Navigate. The coordinator checks the immutable exact-origin envelope.
    Navigate {
        /// Validated web destination.
        url: BrowserUrl,
    },
    /// Semantic inspection; observations remain untrusted, sensitive page data.
    Snapshot {
        /// Requested node ceiling, from one to 1,024.
        max_nodes: u16,
    },
    /// Capture the current viewport; bytes stay on a private bounded transfer.
    Screenshot {
        /// Complete PNG ceiling, at most four MiB.
        max_bytes: u32,
    },
    /// Upload only an existing owned artifact; actual bytes pass pre-effect policy.
    Upload {
        /// Exact current file input from the latest accepted snapshot.
        element: BrowserElementRef,
        /// Existing opaque application-owned RunInput artifact.
        artifact_id: String,
        /// Complete private byte ceiling.
        max_bytes: u32,
    },
    /// Download one current link; no caller-selected native path or URL.
    Download {
        /// Exact current download link from the latest accepted snapshot.
        element: BrowserElementRef,
        /// Complete private byte ceiling.
        max_bytes: u32,
    },
    /// Click one current element.
    Click {
        /// Exact element reference.
        element: BrowserElementRef,
    },
    /// Fill an ordinary field; native drivers must reject credential/password targets.
    Fill {
        /// Exact element reference.
        element: BrowserElementRef,
        /// Ordinary text, bounded to 8 KiB. Unknown secrets cannot be inferred reliably.
        text: String,
    },
    /// Choose options on a current control.
    Select {
        /// Exact element reference.
        element: BrowserElementRef,
        /// One to 32 option values, each bounded to 1 KiB.
        values: Vec<String>,
    },
    /// Press one guest key; this can perform website effects.
    Press {
        /// Restricted key.
        key: BrowserKey,
    },
    /// Scroll the guest, potentially triggering website effects.
    Scroll {
        /// Horizontal distance, bounded to +/- 10,000 CSS pixels.
        x: i32,
        /// Vertical distance, bounded to +/- 10,000 CSS pixels.
        y: i32,
    },
    /// Wait without extending session or network authority.
    Wait {
        /// Closed predicate.
        condition: BrowserWaitCondition,
        /// One to 30,000 milliseconds.
        timeout_ms: u32,
    },
    /// Navigate back with the same egress checks.
    Back {},
    /// Navigate forward with the same egress checks.
    Forward {},
    /// Reload with the same egress checks.
    Reload {},
    /// Stop a pending navigation.
    Stop {},
    /// Open a new tab within the immutable session envelope.
    TabOpen {
        /// Optional initial destination; absent means an isolated blank document.
        url: Option<BrowserUrl>,
    },
    /// Select an already owned tab.
    TabSelect {
        /// Opaque owned tab.
        tab_id: BrowserTabId,
    },
    /// Close an already owned tab.
    TabClose {
        /// Opaque owned tab.
        tab_id: BrowserTabId,
    },
}

impl BrowserAction {
    /// Exact capability identity; observation does not authorize later mutation.
    pub fn kind(&self) -> BrowserActionKind {
        match self {
            Self::Navigate { .. } => BrowserActionKind::Navigate,
            Self::Snapshot { .. } => BrowserActionKind::Snapshot,
            Self::Screenshot { .. } => BrowserActionKind::Screenshot,
            Self::Upload { .. } => BrowserActionKind::Upload,
            Self::Download { .. } => BrowserActionKind::Download,
            Self::Click { .. } => BrowserActionKind::Click,
            Self::Fill { .. } => BrowserActionKind::Fill,
            Self::Select { .. } => BrowserActionKind::Select,
            Self::Press { .. } => BrowserActionKind::Press,
            Self::Scroll { .. } => BrowserActionKind::Scroll,
            Self::Wait { .. } => BrowserActionKind::Wait,
            Self::Back {} => BrowserActionKind::Back,
            Self::Forward {} => BrowserActionKind::Forward,
            Self::Reload {} => BrowserActionKind::Reload,
            Self::Stop {} => BrowserActionKind::Stop,
            Self::TabOpen { .. } => BrowserActionKind::TabOpen,
            Self::TabSelect { .. } => BrowserActionKind::TabSelect,
            Self::TabClose { .. } => BrowserActionKind::TabClose,
        }
    }

    /// Reference requiring current snapshot membership, when applicable.
    pub fn element(&self) -> Option<&BrowserElementRef> {
        match self {
            Self::Click { element }
            | Self::Upload { element, .. }
            | Self::Download { element, .. }
            | Self::Fill { element, .. }
            | Self::Select { element, .. }
            | Self::Wait {
                condition: BrowserWaitCondition::ElementVisible { element },
                ..
            } => Some(element),
            _ => None,
        }
    }
}
