//! Bounded screenshot coordinates and presentation-only page annotations.
use super::BrowserElementRef;
use serde::{Deserialize, Serialize};

/// Physical viewport dimensions from a current browser screenshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewport {
    /// Screenshot width in pixels, from one to 4,096.
    pub width: u16,
    /// Screenshot height in pixels, from one to 4,096.
    pub height: u16,
}

/// One point in the exact supplied screenshot viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserPoint {
    /// Horizontal pixel position, strictly below viewport width.
    pub x: u16,
    /// Vertical pixel position, strictly below viewport height.
    pub y: u16,
}

/// Closed pointer button selection; this never targets the operating-system mouse.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMouseButton {
    /// Primary pointer button.
    #[default]
    Left,
    /// Middle pointer button.
    Middle,
    /// Secondary pointer button.
    Right,
}

/// Screenshot-relative annotation geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRectangle {
    /// Left pixel position.
    pub x: u16,
    /// Top pixel position.
    pub y: u16,
    /// Positive width, contained by the admitted screenshot viewport.
    pub width: u16,
    /// Positive height, contained by the admitted screenshot viewport.
    pub height: u16,
}

/// A current semantic element or bounded screenshot rectangle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserAnnotationTarget {
    /// Resolve geometry from a fresh snapshot handle.
    Element {
        /// Same-document, current-snapshot element.
        element: BrowserElementRef,
    },
    /// Use geometry from the exact supplied screenshot dimensions.
    Rectangle {
        /// Bounded region in physical viewport pixels.
        rect: BrowserRectangle,
    },
}

/// Native annotation palette, distinct from page-authored styles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAnnotationColor {
    /// Blue outline and label.
    #[default]
    Blue,
    /// Red outline and label.
    Red,
    /// Green outline and label.
    Green,
    /// Yellow outline and label.
    Yellow,
}

/// One presentation mark; labels are plain text and never page scripts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserAnnotation {
    /// Native geometry binding.
    pub target: BrowserAnnotationTarget,
    /// Plain label bounded to 160 UTF-8 bytes.
    pub label: String,
    /// Native marker color.
    #[serde(default)]
    pub color: BrowserAnnotationColor,
}
