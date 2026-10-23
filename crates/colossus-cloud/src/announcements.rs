//! Instance-wide administrator announcements and account-owned acknowledgement state.
//!
//! These records never enter runtime journals or grant execution authority. Transport
//! authentication supplies the actor; presentation and Markdown interpretation belong
//! to the host and shared UI.
mod service;
#[cfg(test)]
mod tests;

use crate::{CloudError, CloudResult};
use serde::{Deserialize, Serialize};

/// Maximum retained UTF-8 Markdown body, matching the shared renderer's budget.
pub const MAX_ANNOUNCEMENT_BYTES: usize = 16_384;

/// Shared semantic presentation tone, independent of a host's active palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnouncementTone {
    /// Ordinary feature or operational information.
    Info,
    /// A successful rollout or completed maintenance.
    Success,
    /// A change requiring attention.
    Warning,
    /// An urgent operational announcement.
    Danger,
}

/// Explicit publication lifecycle; drafts and archives are administrator-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnouncementStatus {
    /// Retained work that is absent from the authenticated feed.
    Draft,
    /// Published content, visible until its optional expiration.
    Published,
    /// Withdrawn retained content, absent from the authenticated feed.
    Archived,
}
impl AnnouncementStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Archived => "archived",
        }
    }
}

/// Released announcement metadata and bounded Markdown content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Announcement {
    /// Stable client allocation identity; never a route or credential.
    pub id: String,
    /// Optimistic record revision, including metadata-only updates.
    pub revision: u64,
    /// Content publication revision; metadata edits preserve acknowledgement state.
    pub publication_revision: u64,
    /// Plain-text heading of at most 120 characters.
    pub title: String,
    /// Plain-text banner summary of at most 240 characters.
    pub summary: String,
    /// At most 16 KiB of UTF-8 Markdown, interpreted only by the safe UI renderer.
    pub body: String,
    /// Palette-independent semantic tone.
    pub tone: AnnouncementTone,
    /// Whether an undismissed active item is eligible for the compact global banner.
    pub banner: bool,
    /// Banner ordering priority, between zero and 100 inclusive.
    pub priority: u8,
    /// Explicit administrator-controlled visibility state.
    pub status: AnnouncementStatus,
    /// UTC time of the latest content publication, absent before first publication.
    pub published_at: Option<String>,
    /// Optional UTC expiration; expired items leave the active feed.
    pub expires_at: Option<String>,
    /// Server-owned UTC creation time.
    pub created_at: String,
    /// Server-owned UTC record update time.
    pub updated_at: String,
}
impl Announcement {
    /// Active publication status at an explicit UTC Unix-second observation.
    pub fn active_at(&self, now: i64) -> bool {
        self.status == AnnouncementStatus::Published
            && self.publication_revision > 0
            && self.expires_at.as_ref().is_none_or(|value| {
                parse_timestamp(value).is_ok_and(|expires| expires.unix_timestamp() > now)
            })
    }
}

/// Administrator creation action; creating an archive is not a publication operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateAnnouncementAction {
    /// Retain an unpublished draft.
    Draft,
    /// Publish the initial content.
    Publish,
}

/// Explicit administrator update action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAnnouncementAction {
    /// Save a draft, withdrawing any current publication until it is published again.
    Draft,
    /// Publish the reviewed content; only content changes reset acknowledgement state.
    Publish,
    /// Withdraw the retained announcement.
    Archive,
}

/// Exact first allocation. Retain this request and ID to reconcile an unknown response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAnnouncement {
    /// Client-generated UUID, stable across retries of this allocation.
    pub id: String,
    /// Plain-text heading.
    pub title: String,
    /// Plain-text compact summary.
    pub summary: String,
    /// Bounded Markdown body.
    pub body: String,
    /// Semantic presentation tone.
    pub tone: AnnouncementTone,
    /// Compact banner eligibility.
    pub banner: bool,
    /// Banner priority from zero through 100.
    pub priority: u8,
    /// Optional RFC 3339 expiration, normalized to UTC.
    pub expires_at: Option<String>,
    /// Explicit initial publication action.
    pub action: CreateAnnouncementAction,
}

/// Exact optimistic administrator edit. A stale differing edit always conflicts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAnnouncement {
    /// Exact reviewed record revision.
    pub revision: u64,
    /// Plain-text heading.
    pub title: String,
    /// Plain-text compact summary.
    pub summary: String,
    /// Bounded Markdown body.
    pub body: String,
    /// Semantic presentation tone.
    pub tone: AnnouncementTone,
    /// Compact banner eligibility.
    pub banner: bool,
    /// Banner priority from zero through 100.
    pub priority: u8,
    /// Optional RFC 3339 expiration, normalized to UTC.
    pub expires_at: Option<String>,
    /// Explicit save, publication or withdrawal action.
    pub action: UpdateAnnouncementAction,
}

/// Account-owned monotonic acknowledgement; the caller cannot nominate another user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeAnnouncement {
    /// Exact content publication that was displayed to the user.
    pub publication_revision: u64,
    /// When present, must be true. Reading never reverses another browser's receipt.
    pub read: Option<bool>,
    /// When present, must be true. Dismissal also marks that content read.
    pub dismissed: Option<bool>,
}

/// Per-account released acknowledgement flags for one publication revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AnnouncementAcknowledgement {
    /// Publication revision represented by these flags.
    pub publication_revision: u64,
    /// The caller has opened or dismissed this content revision.
    pub read: bool,
    /// The caller dismissed the compact banner for this content revision.
    pub dismissed: bool,
}

/// One active feed entry with flags scoped to the authenticated account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AnnouncementEntry {
    /// Published, unexpired announcement.
    pub announcement: Announcement,
    /// Caller-specific read flag.
    pub read: bool,
    /// Caller-specific compact-banner dismissal flag.
    pub dismissed: bool,
}

/// Bounded authenticated publication page. Counts describe only loaded items.
#[derive(Clone, Debug, Serialize)]
pub struct AnnouncementPage {
    /// Highest-priority active banner not dismissed by this account, independently
    /// selected across the complete active catalog before the history page limit.
    pub active_banner: Option<AnnouncementEntry>,
    /// Active publications, including items this caller has already dismissed.
    pub announcements: Vec<AnnouncementEntry>,
    /// Exclusive server cursor for another bounded page.
    pub next_cursor: Option<String>,
}

/// Bounded administrator catalog including drafts and archived publications.
#[derive(Clone, Debug, Serialize)]
pub struct AdminAnnouncementPage {
    /// Reviewed global announcement records.
    pub announcements: Vec<Announcement>,
    /// Exclusive server cursor for another bounded page.
    pub next_cursor: Option<String>,
}

/// Internal auditable persistence record; retry fingerprints never enter the UI DTO.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AnnouncementRecord {
    /// Current content and metadata.
    pub announcement: Announcement,
    /// Exact normalized initial request and actor fingerprint.
    pub creation_sha256: String,
    /// Exact normalized latest edit and actor fingerprint, when edited.
    pub last_change_sha256: Option<String>,
    /// Most recently published content fingerprint, preserved while saving a draft.
    pub published_content_sha256: Option<String>,
}

/// Internal account-owned receipt, retained independently of browser storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AnnouncementReceipt {
    /// Announcement identity, bound to the entity key.
    pub announcement_id: String,
    /// Account identity, bound to the key parent and authenticated actor.
    pub user_id: String,
    /// Optimistic receipt record revision.
    pub revision: u64,
    /// Highest publication read by this account.
    pub read_revision: u64,
    /// Highest publication whose compact banner was dismissed by this account.
    pub dismissed_revision: u64,
    /// Server-owned UTC update time.
    pub updated_at: String,
}

pub(crate) fn parse_timestamp(value: &str) -> CloudResult<time::OffsetDateTime> {
    if value.len() > 64 {
        return Err(CloudError::InvalidArgument);
    }
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| CloudError::InvalidArgument)
}
