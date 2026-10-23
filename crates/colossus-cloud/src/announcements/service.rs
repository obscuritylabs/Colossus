use super::*;
use crate::{
    CloudRepository, CloudUser, IDENTITY_NAMESPACE,
    storage::{CloudTransaction, EntityKey, EntityKind, EntityMutation, EntityOrder, EntityQuery},
};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

fn actor(user: &CloudUser, administrator: bool) -> CloudResult<()> {
    crate::validate_identifier(&user.id)?;
    if !user.active || (administrator && !user.is_admin) {
        return Err(CloudError::PermissionDenied);
    }
    Ok(())
}
fn id(value: &str) -> CloudResult<()> {
    let parsed = uuid::Uuid::parse_str(value).map_err(|_| CloudError::InvalidArgument)?;
    if parsed.is_nil() || (parsed.to_string() != value && parsed.simple().to_string() != value) {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}
fn key(id: &str) -> EntityKey {
    EntityKey {
        kind: EntityKind::Announcement,
        project_id: IDENTITY_NAMESPACE.into(),
        parent_id: None,
        id: id.into(),
    }
}
fn receipt_key(user: &str, id: &str) -> EntityKey {
    EntityKey {
        kind: EntityKind::AnnouncementReceipt,
        project_id: IDENTITY_NAMESPACE.into(),
        parent_id: Some(user.into()),
        id: id.into(),
    }
}
fn now() -> CloudResult<String> {
    OffsetDateTime::now_utc()
        // Fixed UTC precision makes the retained publication text an exact,
        // lexically ordered SQL key without losing nanoseconds in a cast.
        .format(
            &time::format_description::parse_borrowed::<2>(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:9]Z",
            )
            .map_err(|_| CloudError::Storage)?,
        )
        .map_err(|_| CloudError::Storage)
}
fn normalize(
    title: &mut String,
    summary: &mut String,
    body: &str,
    priority: u8,
    expires_at: &mut Option<String>,
) -> CloudResult<()> {
    if title.chars().count() > 120
        || title.trim().is_empty()
        || title.chars().any(char::is_control)
        || summary.chars().count() > 240
        || summary.chars().any(char::is_control)
        || body.len() > MAX_ANNOUNCEMENT_BYTES
        || body.trim().is_empty()
        || body
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        || priority > 100
    {
        return Err(CloudError::InvalidArgument);
    }
    *title = title.trim().into();
    *summary = summary.trim().into();
    *expires_at = expires_at
        .as_ref()
        .map(|value| {
            parse_timestamp(value)?
                .to_offset(UtcOffset::UTC)
                .format(&Rfc3339)
                .map_err(|_| CloudError::InvalidArgument)
        })
        .transpose()?;
    Ok(())
}
fn fingerprint(value: &impl Serialize) -> CloudResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| crate::validation::fingerprint(&bytes))
        .map_err(|_| CloudError::InvalidArgument)
}
fn content_fingerprint(announcement: &Announcement) -> CloudResult<String> {
    fingerprint(&(
        &announcement.title,
        &announcement.summary,
        &announcement.body,
        announcement.tone,
    ))
}
fn query(after: Option<&str>, limit: usize, active: bool) -> CloudResult<EntityQuery> {
    if !(1..=100).contains(&limit) {
        return Err(CloudError::InvalidArgument);
    }
    let mut query = EntityQuery::new(EntityKind::Announcement, IDENTITY_NAMESPACE.into());
    query.after = after.map(str::to_owned);
    query.limit = limit;
    query.order = if active {
        EntityOrder::AnnouncementPriority
    } else {
        EntityOrder::CreatedDesc
    };
    query.status = active.then(|| "active".into());
    crate::storage::decode_page_cursor(&query)?;
    Ok(query)
}
fn acknowledgement(
    receipt: Option<&AnnouncementReceipt>,
    publication_revision: u64,
) -> AnnouncementAcknowledgement {
    AnnouncementAcknowledgement {
        publication_revision,
        read: receipt.is_some_and(|value| value.read_revision >= publication_revision),
        dismissed: receipt.is_some_and(|value| value.dismissed_revision >= publication_revision),
    }
}

impl CloudRepository {
    async fn announcement_record(&self, announcement_id: &str) -> CloudResult<AnnouncementRecord> {
        id(announcement_id)?;
        self.store
            .read(&key(announcement_id))
            .await?
            .value
            .try_into()
    }
    async fn announcement_receipt(
        &self,
        user_id: &str,
        announcement_id: &str,
    ) -> CloudResult<Option<AnnouncementReceipt>> {
        match self
            .store
            .read(&receipt_key(user_id, announcement_id))
            .await
        {
            Ok(record) => Ok(Some(record.value.try_into()?)),
            Err(CloudError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }
    /// Read a bounded active publication page for the authenticated account.
    /// Expiration and lifecycle filtering happen in storage before pagination.
    pub async fn announcements(
        &self,
        user: &CloudUser,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<AnnouncementPage> {
        actor(user, false)?;
        let records = self.store.list(&query(after, limit, true)?).await?;
        let mut banner_query = query(None, 1, true)?;
        banner_query.status = Some("active_banner".into());
        banner_query.visible_to = Some(user.id.clone());
        let active_banner = if let Some(record) = self.store.list(&banner_query).await?.pop() {
            let stored: AnnouncementRecord = record.value.try_into()?;
            let announcement = stored.announcement;
            let receipt = self
                .announcement_receipt(&user.id, &announcement.id)
                .await?;
            let flags = acknowledgement(receipt.as_ref(), announcement.publication_revision);
            // A concurrent dismissal can win after selection. Never briefly
            // redisplay that dismissed banner while the next poll reconciles.
            (!flags.dismissed).then_some(AnnouncementEntry {
                announcement,
                read: flags.read,
                dismissed: flags.dismissed,
            })
        } else {
            None
        };
        let next_cursor = (records.len() == limit)
            .then(|| records.last().and_then(|record| record.page_cursor.clone()))
            .flatten();
        let mut announcements = Vec::with_capacity(records.len());
        for record in records {
            let stored: AnnouncementRecord = record.value.try_into()?;
            let announcement = stored.announcement;
            let receipt = self
                .announcement_receipt(&user.id, &announcement.id)
                .await?;
            let flags = acknowledgement(receipt.as_ref(), announcement.publication_revision);
            announcements.push(AnnouncementEntry {
                announcement,
                read: flags.read,
                dismissed: flags.dismissed,
            });
        }
        Ok(AnnouncementPage {
            active_banner,
            announcements,
            next_cursor,
        })
    }
    /// List retained publication work for a verified instance administrator.
    pub async fn admin_announcements(
        &self,
        user: &CloudUser,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<AdminAnnouncementPage> {
        actor(user, true)?;
        let records = self.store.list(&query(after, limit, false)?).await?;
        let next_cursor = (records.len() == limit)
            .then(|| records.last().and_then(|record| record.page_cursor.clone()))
            .flatten();
        let announcements = records
            .into_iter()
            .map(|record| AnnouncementRecord::try_from(record.value).map(|r| r.announcement))
            .collect::<CloudResult<Vec<_>>>()?;
        Ok(AdminAnnouncementPage {
            announcements,
            next_cursor,
        })
    }
    /// Inspect a draft, active or archived record as a verified instance administrator.
    pub async fn admin_announcement(
        &self,
        user: &CloudUser,
        announcement_id: &str,
    ) -> CloudResult<Announcement> {
        actor(user, true)?;
        Ok(self
            .announcement_record(announcement_id)
            .await?
            .announcement)
    }
    /// Allocate an audited announcement once. An exact actor/request replay returns
    /// its retained allocation; reusing the ID for different content conflicts.
    pub async fn create_announcement(
        &self,
        user: &CloudUser,
        mut input: CreateAnnouncement,
    ) -> CloudResult<Announcement> {
        actor(user, true)?;
        id(&input.id)?;
        normalize(
            &mut input.title,
            &mut input.summary,
            &input.body,
            input.priority,
            &mut input.expires_at,
        )?;
        let creation_sha256 = fingerprint(&(user.id.as_str(), &input))?;
        let timestamp = now()?;
        let published = input.action == CreateAnnouncementAction::Publish;
        let announcement = Announcement {
            id: input.id,
            revision: 1,
            publication_revision: u64::from(published),
            title: input.title,
            summary: input.summary,
            body: input.body,
            tone: input.tone,
            banner: input.banner,
            priority: input.priority,
            status: if published {
                AnnouncementStatus::Published
            } else {
                AnnouncementStatus::Draft
            },
            published_at: published.then(|| timestamp.clone()),
            expires_at: input.expires_at,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        };
        let record = AnnouncementRecord {
            published_content_sha256: published
                .then(|| content_fingerprint(&announcement))
                .transpose()?,
            announcement,
            creation_sha256,
            last_change_sha256: None,
        };
        let result = self
            .store
            .commit(CloudTransaction {
                entities: vec![EntityMutation {
                    key: key(&record.announcement.id),
                    expected_revision: 0,
                    value: record.clone().into(),
                    actor: user.id.clone(),
                    operation: "cloud.announcement.created.v1".into(),
                }],
                ..Default::default()
            })
            .await;
        match result {
            Ok(()) => Ok(record.announcement),
            Err(colossus_ports::StoreError::Conflict { .. }) => {
                let retained = self.announcement_record(&record.announcement.id).await?;
                if retained.creation_sha256 == record.creation_sha256 {
                    Ok(retained.announcement)
                } else {
                    Err(CloudError::Conflict)
                }
            }
            Err(error) => Err(error.into()),
        }
    }
    /// Apply one reviewed administrator edit. Drafts and archives are not released;
    /// metadata-only publications preserve content acknowledgements.
    pub async fn update_announcement(
        &self,
        user: &CloudUser,
        announcement_id: &str,
        mut input: UpdateAnnouncement,
    ) -> CloudResult<Announcement> {
        actor(user, true)?;
        normalize(
            &mut input.title,
            &mut input.summary,
            &input.body,
            input.priority,
            &mut input.expires_at,
        )?;
        if input.revision == 0 {
            return Err(CloudError::InvalidArgument);
        }
        let change_sha256 = fingerprint(&(user.id.as_str(), announcement_id, &input))?;
        let mut record = self.announcement_record(announcement_id).await?;
        if record.announcement.revision != input.revision {
            return if record.announcement.revision == input.revision.saturating_add(1)
                && record.last_change_sha256.as_ref() == Some(&change_sha256)
            {
                Ok(record.announcement)
            } else {
                Err(CloudError::Conflict)
            };
        }
        let announcement = &mut record.announcement;
        announcement.revision = input
            .revision
            .checked_add(1)
            .ok_or(CloudError::InvalidArgument)?;
        announcement.title = input.title;
        announcement.summary = input.summary;
        announcement.body = input.body;
        announcement.tone = input.tone;
        announcement.banner = input.banner;
        announcement.priority = input.priority;
        announcement.expires_at = input.expires_at;
        announcement.updated_at = now()?;
        announcement.status = match input.action {
            UpdateAnnouncementAction::Draft => AnnouncementStatus::Draft,
            UpdateAnnouncementAction::Archive => AnnouncementStatus::Archived,
            UpdateAnnouncementAction::Publish => {
                let content = content_fingerprint(announcement)?;
                if record.published_content_sha256.as_ref() != Some(&content) {
                    announcement.publication_revision = announcement
                        .publication_revision
                        .checked_add(1)
                        .ok_or(CloudError::InvalidArgument)?;
                    announcement.published_at = Some(announcement.updated_at.clone());
                    record.published_content_sha256 = Some(content);
                }
                AnnouncementStatus::Published
            }
        };
        record.last_change_sha256 = Some(change_sha256);
        let result = self
            .store
            .commit(CloudTransaction {
                entities: vec![EntityMutation {
                    key: key(announcement_id),
                    expected_revision: input.revision,
                    value: record.clone().into(),
                    actor: user.id.clone(),
                    operation: "cloud.announcement.updated.v1".into(),
                }],
                ..Default::default()
            })
            .await;
        match result {
            Ok(()) => Ok(record.announcement),
            Err(error) => {
                // A contended adapter or lost commit acknowledgement can report
                // uncertainty instead of a typed conflict. Never retry the write:
                // release success only when canonical state proves this exact
                // actor/request committed at the one expected next revision.
                if let Ok(retained) = self.announcement_record(announcement_id).await
                    && retained.announcement.revision == record.announcement.revision
                    && retained.last_change_sha256 == record.last_change_sha256
                {
                    return Ok(retained.announcement);
                }
                Err(error.into())
            }
        }
    }
    /// Monotonically retain this account's read/dismiss state for a displayed
    /// publication. A bounded CAS merge reconciles simultaneous browser tabs.
    pub async fn acknowledge_announcement(
        &self,
        user: &CloudUser,
        announcement_id: &str,
        input: AcknowledgeAnnouncement,
    ) -> CloudResult<AnnouncementAcknowledgement> {
        actor(user, false)?;
        if input.publication_revision == 0
            || input.read == Some(false)
            || input.dismissed == Some(false)
            || (input.read != Some(true) && input.dismissed != Some(true))
        {
            return Err(CloudError::InvalidArgument);
        }
        let announcement = self
            .announcement_record(announcement_id)
            .await?
            .announcement;
        if input.publication_revision > announcement.publication_revision {
            return Err(CloudError::Conflict);
        }
        for _ in 0..4 {
            let previous = self.announcement_receipt(&user.id, announcement_id).await?;
            let expected = previous.as_ref().map_or(0, |record| record.revision);
            let mut receipt = previous.clone().unwrap_or(AnnouncementReceipt {
                announcement_id: announcement_id.into(),
                user_id: user.id.clone(),
                revision: 0,
                read_revision: 0,
                dismissed_revision: 0,
                updated_at: String::new(),
            });
            receipt.read_revision = receipt.read_revision.max(input.publication_revision);
            if input.dismissed == Some(true) {
                receipt.dismissed_revision =
                    receipt.dismissed_revision.max(input.publication_revision);
            }
            if previous.as_ref().is_some_and(|previous| {
                previous.read_revision == receipt.read_revision
                    && previous.dismissed_revision == receipt.dismissed_revision
            }) {
                return Ok(acknowledgement(Some(&receipt), input.publication_revision));
            }
            receipt.revision = expected.checked_add(1).ok_or(CloudError::Storage)?;
            receipt.updated_at = now()?;
            let result = self
                .store
                .commit(CloudTransaction {
                    entities: vec![EntityMutation {
                        key: receipt_key(&user.id, announcement_id),
                        expected_revision: expected,
                        value: receipt.clone().into(),
                        actor: user.id.clone(),
                        operation: "cloud.announcement.acknowledged.v1".into(),
                    }],
                    ..Default::default()
                })
                .await;
            match result {
                Ok(()) => {
                    return Ok(acknowledgement(Some(&receipt), input.publication_revision));
                }
                Err(colossus_ports::StoreError::Conflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(CloudError::Conflict)
    }
}
