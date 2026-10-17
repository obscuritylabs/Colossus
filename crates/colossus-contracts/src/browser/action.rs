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
    /// Evaluate a bounded expression with native side-effect checking enabled.
    Evaluate,
    /// Inspect a current owned page through a closed CDP method list.
    Cdp,
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
    /// Move the guest pointer over a current element.
    Hover,
    /// Double-click a current element.
    DoubleClick,
    /// Right-click a current element.
    RightClick,
    /// Drag between two current elements.
    Drag,
    /// Click screenshot-relative coordinates in a current viewport.
    ClickAt,
    /// Drag a bounded screenshot-relative path.
    DragAt,
    /// Replace native presentation annotations.
    Annotate,
    /// Remove native presentation annotations.
    AnnotationClear,
    /// Capture the current viewport with its admitted annotations.
    AnnotatedScreenshot,
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
    /// Evaluate a side-effect-free expression in the current owned document.
    Evaluate {
        /// Exact latest semantic snapshot.
        snapshot_id: BrowserSnapshotId,
        /// JavaScript expression, bounded to8KiB.
        expression: String,
    },
    /// Execute one bounded page-inspection CDP method.
    Cdp {
        /// Exact latest semantic snapshot.
        snapshot_id: BrowserSnapshotId,
        /// Closed page-inspection method.
        method: super::BrowserCdpMethod,
        /// Bounded JSON object of native method parameters.
        params: serde_json::Value,
    },
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
    /// Move the guest pointer over a fresh semantic target.
    Hover {
        /// Exact element reference.
        element: BrowserElementRef,
    },
    /// Emit a real double-click sequence for a fresh semantic target.
    DoubleClick {
        /// Exact element reference.
        element: BrowserElementRef,
    },
    /// Emit a secondary click for a fresh semantic target.
    RightClick {
        /// Exact element reference.
        element: BrowserElementRef,
    },
    /// Drag with the primary pointer button between two fresh targets.
    Drag {
        /// Exact source reference.
        element: BrowserElementRef,
        /// Exact destination reference in the same snapshot.
        to: BrowserElementRef,
    },
    /// Click only inside the exact supplied screenshot viewport.
    ClickAt {
        /// Current accepted semantic snapshot.
        snapshot_id: BrowserSnapshotId,
        /// Physical dimensions of the inspected screenshot.
        viewport: super::BrowserViewport,
        /// Screenshot-relative point.
        point: super::BrowserPoint,
        /// Guest pointer button.
        #[serde(default)]
        button: super::BrowserMouseButton,
        /// One or two clicks.
        click_count: u8,
    },
    /// Drag a finite path inside the exact supplied screenshot viewport.
    DragAt {
        /// Current accepted semantic snapshot.
        snapshot_id: BrowserSnapshotId,
        /// Physical dimensions of the inspected screenshot.
        viewport: super::BrowserViewport,
        /// Two to 64 bounded points, from press to release.
        path: Vec<super::BrowserPoint>,
    },
    /// Replace native marks without modifying the DOM or browser input.
    Annotate {
        /// Current accepted semantic snapshot.
        snapshot_id: BrowserSnapshotId,
        /// Physical dimensions of the inspected screenshot.
        viewport: super::BrowserViewport,
        /// One to 16 marks in the same document and snapshot.
        annotations: Vec<super::BrowserAnnotation>,
        /// Original mark lifetime, from one to 300,000 milliseconds.
        ttl_ms: u32,
    },
    /// Clear native marks without modifying the document.
    AnnotationClear {},
    /// Capture annotated pixels through the same private artifact custody.
    AnnotatedScreenshot {
        /// Complete PNG byte ceiling.
        max_bytes: u32,
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
            Self::Evaluate { .. } => BrowserActionKind::Evaluate,
            Self::Cdp { .. } => BrowserActionKind::Cdp,
            Self::Navigate { .. } => BrowserActionKind::Navigate,
            Self::Snapshot { .. } => BrowserActionKind::Snapshot,
            Self::Screenshot { .. } => BrowserActionKind::Screenshot,
            Self::Upload { .. } => BrowserActionKind::Upload,
            Self::Download { .. } => BrowserActionKind::Download,
            Self::Click { .. } => BrowserActionKind::Click,
            Self::Hover { .. } => BrowserActionKind::Hover,
            Self::DoubleClick { .. } => BrowserActionKind::DoubleClick,
            Self::RightClick { .. } => BrowserActionKind::RightClick,
            Self::Drag { .. } => BrowserActionKind::Drag,
            Self::ClickAt { .. } => BrowserActionKind::ClickAt,
            Self::DragAt { .. } => BrowserActionKind::DragAt,
            Self::Annotate { .. } => BrowserActionKind::Annotate,
            Self::AnnotationClear {} => BrowserActionKind::AnnotationClear,
            Self::AnnotatedScreenshot { .. } => BrowserActionKind::AnnotatedScreenshot,
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
            | Self::Hover { element }
            | Self::DoubleClick { element }
            | Self::RightClick { element }
            | Self::Drag { element, .. }
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

    /// Every reference that must belong to the current accepted snapshot.
    pub fn elements(&self) -> Vec<&BrowserElementRef> {
        match self {
            Self::Drag { element, to } => vec![element, to],
            Self::Annotate { annotations, .. } => annotations
                .iter()
                .filter_map(|annotation| match &annotation.target {
                    super::BrowserAnnotationTarget::Element { element } => Some(element),
                    super::BrowserAnnotationTarget::Rectangle { .. } => None,
                })
                .collect(),
            _ => self.element().into_iter().collect(),
        }
    }

    /// Coordinate-based effects also retain the current semantic observation fence.
    pub fn coordinate_snapshot(&self) -> Option<&BrowserSnapshotId> {
        match self {
            Self::ClickAt { snapshot_id, .. }
            | Self::Evaluate { snapshot_id, .. }
            | Self::Cdp { snapshot_id, .. }
            | Self::DragAt { snapshot_id, .. }
            | Self::Annotate { snapshot_id, .. } => Some(snapshot_id),
            _ => None,
        }
    }
}
