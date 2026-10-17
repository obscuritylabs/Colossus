//! Bounded developer inspection of one owned document.
use serde::{Deserialize, Serialize};
/// The page-scoped CDP inspection methods supported by the owned adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BrowserCdpMethod {
    /// Read the current viewport metrics.
    #[serde(rename = "Page.getLayoutMetrics")]
    LayoutMetrics,
    /// Read browser performance counters for this page.
    #[serde(rename = "Performance.getMetrics")]
    PerformanceMetrics,
    /// Read the owned DOM tree; handles remain scoped to this owned document.
    #[serde(rename = "DOM.getDocument")]
    Document,
    /// Resolve one selector inside this owned document.
    #[serde(rename = "DOM.querySelector")]
    QuerySelector,
    /// Resolve a bounded selector result inside this owned document.
    #[serde(rename = "DOM.querySelectorAll")]
    QuerySelectorAll,
    /// Read one owned node's attributes.
    #[serde(rename = "DOM.getAttributes")]
    Attributes,
    /// Read one owned node's box model.
    #[serde(rename = "DOM.getBoxModel")]
    BoxModel,
    /// Read one owned node's HTML.
    #[serde(rename = "DOM.getOuterHTML")]
    OuterHtml,
    /// Read one owned node's computed styles.
    #[serde(rename = "CSS.getComputedStyleForNode")]
    ComputedStyle,
    /// Read active media queries.
    #[serde(rename = "CSS.getMediaQueries")]
    MediaQueries,
}
impl BrowserCdpMethod {
    /// Native method name, selected from this closed list only.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LayoutMetrics => "Page.getLayoutMetrics",
            Self::PerformanceMetrics => "Performance.getMetrics",
            Self::Document => "DOM.getDocument",
            Self::QuerySelector => "DOM.querySelector",
            Self::QuerySelectorAll => "DOM.querySelectorAll",
            Self::Attributes => "DOM.getAttributes",
            Self::BoxModel => "DOM.getBoxModel",
            Self::OuterHtml => "DOM.getOuterHTML",
            Self::ComputedStyle => "CSS.getComputedStyleForNode",
            Self::MediaQueries => "CSS.getMediaQueries",
        }
    }
}
