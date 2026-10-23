//! Native announcement columns and explicit domain conversion.
use super::{DomainRow, Metadata, mismatch, signed, unsigned};
use colossus_cloud::{
    CloudError, CloudResult,
    announcements::{
        Announcement, AnnouncementReceipt, AnnouncementRecord, AnnouncementStatus, AnnouncementTone,
    },
    storage::EntityValue,
};
use colossus_ports::StoreError;
use diesel::{
    QueryableByName,
    sql_types::{BigInt, Bool, Nullable, Text},
};

row!(AnnouncementRow {
    publication_revision: i64 => BigInt,
    title: String => Text,
    summary: String => Text,
    body: String => Text,
    tone: String => Text,
    banner: bool => Bool,
    priority: i64 => BigInt,
    publication_status: String => Text,
    published_at: Option<String> => Nullable<Text>,
    expires_at: Option<String> => Nullable<Text>,
    announcement_created_at: String => Text,
    announcement_updated_at: String => Text,
    creation_sha256: String => Text,
    last_change_sha256: Option<String> => Nullable<Text>,
    published_content_sha256: Option<String> => Nullable<Text>,
});

row!(AnnouncementReceiptRow {
    read_revision: i64 => BigInt,
    dismissed_revision: i64 => BigInt,
    receipt_updated_at: String => Text,
});

fn tone_name(value: AnnouncementTone) -> &'static str {
    match value {
        AnnouncementTone::Info => "info",
        AnnouncementTone::Success => "success",
        AnnouncementTone::Warning => "warning",
        AnnouncementTone::Danger => "danger",
    }
}
fn tone(value: &str) -> CloudResult<AnnouncementTone> {
    match value {
        "info" => Ok(AnnouncementTone::Info),
        "success" => Ok(AnnouncementTone::Success),
        "warning" => Ok(AnnouncementTone::Warning),
        "danger" => Ok(AnnouncementTone::Danger),
        _ => Err(CloudError::Storage),
    }
}
fn status(value: &str) -> CloudResult<AnnouncementStatus> {
    match value {
        "draft" => Ok(AnnouncementStatus::Draft),
        "published" => Ok(AnnouncementStatus::Published),
        "archived" => Ok(AnnouncementStatus::Archived),
        _ => Err(CloudError::Storage),
    }
}

impl DomainRow for AnnouncementRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Announcement(record) = value else {
            return Err(mismatch());
        };
        let value = &record.announcement;
        Ok(Self {
            metadata,
            publication_revision: signed(value.publication_revision)?,
            title: value.title.clone(),
            summary: value.summary.clone(),
            body: value.body.clone(),
            tone: tone_name(value.tone).into(),
            banner: value.banner,
            priority: value.priority.into(),
            publication_status: match value.status {
                AnnouncementStatus::Draft => "draft",
                AnnouncementStatus::Published => "published",
                AnnouncementStatus::Archived => "archived",
            }
            .into(),
            published_at: value.published_at.clone(),
            expires_at: value.expires_at.clone(),
            announcement_created_at: value.created_at.clone(),
            announcement_updated_at: value.updated_at.clone(),
            creation_sha256: record.creation_sha256.clone(),
            last_change_sha256: record.last_change_sha256.clone(),
            published_content_sha256: record.published_content_sha256.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(AnnouncementRecord {
            announcement: Announcement {
                id: self.metadata.id.clone(),
                revision: self.metadata.revision()?,
                publication_revision: unsigned(self.publication_revision)?,
                title: self.title,
                summary: self.summary,
                body: self.body,
                tone: tone(&self.tone)?,
                banner: self.banner,
                priority: u8::try_from(self.priority).map_err(|_| CloudError::Storage)?,
                status: status(&self.publication_status)?,
                published_at: self.published_at,
                expires_at: self.expires_at,
                created_at: self.announcement_created_at,
                updated_at: self.announcement_updated_at,
            },
            creation_sha256: self.creation_sha256,
            last_change_sha256: self.last_change_sha256,
            published_content_sha256: self.published_content_sha256,
        }
        .into())
    }
}

impl DomainRow for AnnouncementReceiptRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::AnnouncementReceipt(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            read_revision: signed(value.read_revision)?,
            dismissed_revision: signed(value.dismissed_revision)?,
            receipt_updated_at: value.updated_at.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(AnnouncementReceipt {
            announcement_id: self.metadata.id.clone(),
            user_id: self.metadata.parent_id.clone(),
            revision: self.metadata.revision()?,
            read_revision: unsigned(self.read_revision)?,
            dismissed_revision: unsigned(self.dismissed_revision)?,
            updated_at: self.receipt_updated_at,
        }
        .into())
    }
}
