//! Caller-visible runtime discovery with explicit local ownership flags.
use crate::{PageResponse, Run};

/// Runtime-derived authority for one released owned or shared run.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serialization",
    derive(serde::Serialize, serde::Deserialize)
)]
#[cfg_attr(feature = "serialization", serde(deny_unknown_fields))]
pub struct VisibleRun {
    /// Released projection of one durable source run.
    pub run: Run,
    /// The authenticated caller owns cancellation and response authority.
    pub controllable: bool,
    /// The caller may continue this session using its own execution grant.
    pub continuable: bool,
}

/// One bounded source-stable discovery page.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serialization",
    derive(serde::Serialize, serde::Deserialize)
)]
#[cfg_attr(feature = "serialization", serde(deny_unknown_fields))]
pub struct ListVisibleRunsResponse {
    /// Caller-owned or explicitly shared released runs.
    pub runs: Vec<VisibleRun>,
    /// Opaque authenticated continuation cursor.
    pub page: Option<PageResponse>,
}
