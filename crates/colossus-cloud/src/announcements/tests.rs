use super::*;
use crate::{CloudRepository, CloudUser, storage::MemoryCloudStore};
use std::sync::Arc;
#[path = "concurrent_store.rs"]
mod concurrent_store;

fn user(id: &str, is_admin: bool) -> CloudUser {
    CloudUser {
        id: id.into(),
        display_name: id.into(),
        email: None,
        active: true,
        is_admin,
        revision: 1,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        identities: Vec::new(),
    }
}
fn create(n: u128, action: CreateAnnouncementAction) -> CreateAnnouncement {
    CreateAnnouncement {
        id: uuid::Uuid::from_u128(n).to_string(),
        title: " Feature update ".into(),
        summary: " A compact summary ".into(),
        body: "## New features\n\n**Shared UI** and `code`.\n".into(),
        tone: AnnouncementTone::Info,
        banner: true,
        priority: 50,
        expires_at: None,
        action,
    }
}
fn edit(value: &Announcement, action: UpdateAnnouncementAction) -> UpdateAnnouncement {
    UpdateAnnouncement {
        revision: value.revision,
        title: value.title.clone(),
        summary: value.summary.clone(),
        body: value.body.clone(),
        tone: value.tone,
        banner: value.banner,
        priority: value.priority,
        expires_at: value.expires_at.clone(),
        action,
    }
}
fn dismiss(publication_revision: u64) -> AcknowledgeAnnouncement {
    AcknowledgeAnnouncement {
        publication_revision,
        read: None,
        dismissed: Some(true),
    }
}

#[tokio::test]
async fn announcements_require_global_administration_and_allocate_once() {
    let repo = CloudRepository::new(Arc::new(MemoryCloudStore::default())).unwrap();
    let admin = user("administrator", true);
    let reader = user("reader", false);
    let input = create(1, CreateAnnouncementAction::Draft);
    assert_eq!(
        repo.create_announcement(&reader, input.clone())
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let saved = repo
        .create_announcement(&admin, input.clone())
        .await
        .unwrap();
    assert_eq!(saved.title, "Feature update");
    assert_eq!(saved.summary, "A compact summary");
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.publication_revision, 0);
    assert!(saved.published_at.is_none());
    assert!(
        repo.announcements(&reader, None, 20)
            .await
            .unwrap()
            .announcements
            .is_empty()
    );
    assert_eq!(
        repo.create_announcement(&admin, input.clone())
            .await
            .unwrap(),
        saved
    );
    let mut different = input;
    different.body = "Different content".into();
    assert_eq!(
        repo.create_announcement(&admin, different)
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    assert_eq!(
        repo.admin_announcements(&reader, None, 20)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.admin_announcement(&reader, &saved.id)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let published = repo
        .update_announcement(
            &admin,
            &saved.id,
            edit(&saved, UpdateAnnouncementAction::Publish),
        )
        .await
        .unwrap();
    assert_eq!(published.publication_revision, 1);
    assert!(published.published_at.is_some());
    let page = repo.announcements(&reader, None, 20).await.unwrap();
    assert_eq!(page.announcements[0].announcement, published);
    assert!(!page.announcements[0].read);
    assert!(!page.announcements[0].dismissed);
    let mut disabled = reader;
    disabled.active = false;
    assert_eq!(
        repo.announcements(&disabled, None, 20).await.unwrap_err(),
        CloudError::PermissionDenied
    );
}

#[tokio::test]
async fn announcements_receipts_survive_reopen_and_merge_per_account() {
    let store = Arc::new(MemoryCloudStore::default());
    let repo = CloudRepository::new(store.clone()).unwrap();
    let announcement = repo
        .create_announcement(
            &user("admin", true),
            create(2, CreateAnnouncementAction::Publish),
        )
        .await
        .unwrap();
    let alice = user("alice", false);
    let bob = user("bob", false);
    let read = AcknowledgeAnnouncement {
        publication_revision: 1,
        read: Some(true),
        dismissed: None,
    };
    let (read_result, dismiss_result) = tokio::join!(
        repo.acknowledge_announcement(&alice, &announcement.id, read),
        repo.acknowledge_announcement(&alice, &announcement.id, dismiss(1))
    );
    assert!(read_result.unwrap().read);
    assert!(dismiss_result.unwrap().dismissed);
    let reopened = CloudRepository::new(store).unwrap();
    let entry = reopened
        .announcements(&alice, None, 20)
        .await
        .unwrap()
        .announcements
        .remove(0);
    assert!(entry.read && entry.dismissed);
    let other = reopened
        .announcements(&bob, None, 20)
        .await
        .unwrap()
        .announcements
        .remove(0);
    assert!(!other.read && !other.dismissed);
    assert!(
        reopened
            .acknowledge_announcement(&alice, &announcement.id, dismiss(1))
            .await
            .unwrap()
            .dismissed
    );
    assert_eq!(
        reopened
            .acknowledge_announcement(&alice, &announcement.id, dismiss(2))
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    for invalid in [
        AcknowledgeAnnouncement {
            publication_revision: 0,
            read: Some(true),
            dismissed: None,
        },
        AcknowledgeAnnouncement {
            publication_revision: 1,
            read: Some(false),
            dismissed: None,
        },
        AcknowledgeAnnouncement {
            publication_revision: 1,
            read: None,
            dismissed: Some(false),
        },
        AcknowledgeAnnouncement {
            publication_revision: 1,
            read: None,
            dismissed: None,
        },
    ] {
        assert_eq!(
            reopened
                .acknowledge_announcement(&alice, &announcement.id, invalid)
                .await
                .unwrap_err(),
            CloudError::InvalidArgument
        );
    }
}

#[tokio::test]
async fn announcements_only_new_published_content_resets_seen_state() {
    let repo = CloudRepository::new(Arc::new(MemoryCloudStore::default())).unwrap();
    let admin = user("admin", true);
    let reader = user("reader", false);
    let initial = repo
        .create_announcement(&admin, create(3, CreateAnnouncementAction::Publish))
        .await
        .unwrap();
    repo.acknowledge_announcement(&reader, &initial.id, dismiss(1))
        .await
        .unwrap();
    let mut metadata = edit(&initial, UpdateAnnouncementAction::Publish);
    metadata.priority = 90;
    metadata.banner = false;
    metadata.expires_at = Some("2999-01-01T01:00:00+01:00".into());
    let saved = repo
        .update_announcement(&admin, &initial.id, metadata.clone())
        .await
        .unwrap();
    assert_eq!(saved.publication_revision, 1);
    assert_eq!(saved.published_at, initial.published_at);
    assert_eq!(saved.expires_at.as_deref(), Some("2999-01-01T00:00:00Z"));
    assert!(
        repo.announcements(&reader, None, 20)
            .await
            .unwrap()
            .announcements[0]
            .dismissed
    );
    assert_eq!(
        repo.update_announcement(&admin, &initial.id, metadata)
            .await
            .unwrap(),
        saved
    );
    let mut draft = edit(&saved, UpdateAnnouncementAction::Draft);
    draft.body = "A reviewed second publication".into();
    let draft = repo
        .update_announcement(&admin, &saved.id, draft)
        .await
        .unwrap();
    assert_eq!(draft.publication_revision, 1);
    assert!(
        repo.announcements(&reader, None, 20)
            .await
            .unwrap()
            .announcements
            .is_empty()
    );
    let published = repo
        .update_announcement(
            &admin,
            &draft.id,
            edit(&draft, UpdateAnnouncementAction::Publish),
        )
        .await
        .unwrap();
    assert_eq!(published.publication_revision, 2);
    let entry = repo
        .announcements(&reader, None, 20)
        .await
        .unwrap()
        .announcements
        .remove(0);
    assert!(!entry.read && !entry.dismissed);
    repo.acknowledge_announcement(&reader, &initial.id, dismiss(1))
        .await
        .unwrap();
    assert!(
        !repo
            .announcements(&reader, None, 20)
            .await
            .unwrap()
            .announcements[0]
            .dismissed
    );
    repo.acknowledge_announcement(&reader, &initial.id, dismiss(2))
        .await
        .unwrap();
    let archived = repo
        .update_announcement(
            &admin,
            &published.id,
            edit(&published, UpdateAnnouncementAction::Archive),
        )
        .await
        .unwrap();
    assert!(
        repo.announcements(&reader, None, 20)
            .await
            .unwrap()
            .announcements
            .is_empty()
    );
    let restored = repo
        .update_announcement(
            &admin,
            &archived.id,
            edit(&archived, UpdateAnnouncementAction::Publish),
        )
        .await
        .unwrap();
    assert_eq!(restored.publication_revision, 2);
    assert!(
        repo.announcements(&reader, None, 20)
            .await
            .unwrap()
            .announcements[0]
            .dismissed
    );
}

#[tokio::test]
async fn announcements_filter_expiration_before_bounded_query_bound_pages() {
    let repo = CloudRepository::new(Arc::new(MemoryCloudStore::default())).unwrap();
    let admin = user("admin", true);
    let reader = user("reader", false);
    let old = repo
        .create_announcement(&admin, create(4, CreateAnnouncementAction::Publish))
        .await
        .unwrap();
    let newer = repo
        .create_announcement(&admin, create(5, CreateAnnouncementAction::Publish))
        .await
        .unwrap();
    let mut expired = create(6, CreateAnnouncementAction::Publish);
    expired.expires_at = Some("2000-01-01T00:00:00Z".into());
    repo.create_announcement(&admin, expired).await.unwrap();
    repo.create_announcement(&admin, create(7, CreateAnnouncementAction::Draft))
        .await
        .unwrap();
    let first = repo.announcements(&reader, None, 1).await.unwrap();
    assert_eq!(first.announcements[0].announcement.id, newer.id);
    let second = repo
        .announcements(&reader, first.next_cursor.as_deref(), 1)
        .await
        .unwrap();
    assert_eq!(second.announcements[0].announcement.id, old.id);
    assert!(
        repo.announcements(&reader, second.next_cursor.as_deref(), 1)
            .await
            .unwrap()
            .announcements
            .is_empty()
    );
    let catalog = repo.admin_announcements(&admin, None, 2).await.unwrap();
    assert_eq!(catalog.announcements.len(), 2);
    assert_eq!(
        repo.admin_announcements(&admin, catalog.next_cursor.as_deref(), 2)
            .await
            .unwrap()
            .announcements
            .len(),
        2
    );
    assert_eq!(
        repo.admin_announcements(&admin, first.next_cursor.as_deref(), 2)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
    for limit in [0, 101] {
        assert_eq!(
            repo.announcements(&reader, None, limit).await.unwrap_err(),
            CloudError::InvalidArgument
        );
    }
}

#[tokio::test]
async fn announcements_concurrent_admin_edits_have_one_winner() {
    let repo = CloudRepository::new(Arc::new(MemoryCloudStore::default())).unwrap();
    let admin = user("admin", true);
    let initial = repo
        .create_announcement(&admin, create(8, CreateAnnouncementAction::Publish))
        .await
        .unwrap();
    let mut left = edit(&initial, UpdateAnnouncementAction::Publish);
    left.title = "Left edit".into();
    let mut right = left.clone();
    right.title = "Right edit".into();
    let (left, right) = tokio::join!(
        repo.update_announcement(&admin, &initial.id, left),
        repo.update_announcement(&admin, &initial.id, right)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(left.err().or(right.err()), Some(CloudError::Conflict));
    assert_eq!(
        repo.admin_announcement(&admin, &initial.id)
            .await
            .unwrap()
            .publication_revision,
        2
    );
    assert_eq!(
        repo.update_announcement(
            &admin,
            &initial.id,
            edit(&initial, UpdateAnnouncementAction::Archive)
        )
        .await
        .unwrap_err(),
        CloudError::Conflict
    );
}

#[tokio::test]
async fn announcements_identical_concurrent_updates_reconcile_the_committed_winner() {
    let store = Arc::new(MemoryCloudStore::default());
    let repo = CloudRepository::new(store.clone()).unwrap();
    let admin = user("admin", true);
    let initial = repo
        .create_announcement(&admin, create(10, CreateAnnouncementAction::Publish))
        .await
        .unwrap();
    let repo =
        CloudRepository::new(Arc::new(concurrent_store::ConcurrentUpdates::new(store))).unwrap();
    let mut update = edit(&initial, UpdateAnnouncementAction::Publish);
    update.title = "The same reviewed edit".into();
    // The store barrier forces both requests past their revision-1 read before
    // either writes. One must reconcile a real post-read CAS conflict.
    let (left, right) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(
            repo.update_announcement(&admin, &initial.id, update.clone()),
            repo.update_announcement(&admin, &initial.id, update)
        )
    })
    .await
    .unwrap();
    let left = left.unwrap();
    assert_eq!(right.unwrap(), left);
    assert_eq!(left.revision, 2);
    assert_eq!(left.publication_revision, 2);
}

#[tokio::test]
async fn announcements_banners_and_republications_are_not_hidden_by_history_page_limits() {
    let repo = CloudRepository::new(Arc::new(MemoryCloudStore::default())).unwrap();
    let admin = user("admin", true);
    let reader = user("reader", false);
    let mut old = create(100, CreateAnnouncementAction::Publish);
    old.priority = 10;
    let old = repo.create_announcement(&admin, old).await.unwrap();
    for n in 101..126 {
        let mut input = create(n, CreateAnnouncementAction::Publish);
        input.banner = false;
        input.priority = 100;
        repo.create_announcement(&admin, input).await.unwrap();
    }
    let first = repo.announcements(&reader, None, 20).await.unwrap();
    assert!(
        first
            .announcements
            .iter()
            .all(|entry| !entry.announcement.banner)
    );
    assert_eq!(
        first.active_banner.as_ref().unwrap().announcement.id,
        old.id
    );
    for n in 126..151 {
        let mut input = create(n, CreateAnnouncementAction::Publish);
        input.priority = 100;
        let dismissed = repo.create_announcement(&admin, input).await.unwrap();
        repo.acknowledge_announcement(&reader, &dismissed.id, dismiss(1))
            .await
            .unwrap();
    }
    let page = repo.announcements(&reader, None, 20).await.unwrap();
    assert!(page.announcements.iter().all(|entry| entry.dismissed));
    assert_eq!(page.active_banner.as_ref().unwrap().announcement.id, old.id);
    let mut elevated = edit(&old, UpdateAnnouncementAction::Publish);
    elevated.priority = 100;
    elevated.body.push_str("\nA new content publication.");
    let elevated = repo
        .update_announcement(&admin, &old.id, elevated)
        .await
        .unwrap();
    let page = repo.announcements(&reader, None, 20).await.unwrap();
    assert_eq!(page.announcements[0].announcement.id, old.id);
    assert_eq!(
        page.active_banner
            .as_ref()
            .unwrap()
            .announcement
            .publication_revision,
        2
    );
    let mut identities = std::collections::BTreeSet::new();
    let mut page = page;
    loop {
        for entry in page.announcements {
            assert!(identities.insert(entry.announcement.id));
        }
        let Some(cursor) = page.next_cursor else {
            break;
        };
        page = repo
            .announcements(&reader, Some(&cursor), 20)
            .await
            .unwrap();
    }
    assert_eq!(identities.len(), 51);
    repo.acknowledge_announcement(&reader, &elevated.id, dismiss(2))
        .await
        .unwrap();
    assert!(
        repo.announcements(&reader, None, 20)
            .await
            .unwrap()
            .active_banner
            .is_none()
    );
}

#[tokio::test]
async fn announcements_bound_utf8_plain_text_and_strict_wire_shapes() {
    let repo = CloudRepository::new(Arc::new(MemoryCloudStore::default())).unwrap();
    let admin = user("admin", true);
    let mut input = create(9, CreateAnnouncementAction::Draft);
    input.body = "é".repeat(MAX_ANNOUNCEMENT_BYTES / 2);
    repo.create_announcement(&admin, input.clone())
        .await
        .unwrap();
    let mut invalid = Vec::new();
    let mut over = input.clone();
    over.body.push('é');
    invalid.push(over);
    let mut over = input.clone();
    over.title = "a".repeat(121);
    invalid.push(over);
    let mut over = input.clone();
    over.summary = "a".repeat(241);
    invalid.push(over);
    let mut over = input.clone();
    over.title = "Bad\nheading".into();
    invalid.push(over);
    let mut over = input.clone();
    over.body = "bad\0body".into();
    invalid.push(over);
    let mut over = input.clone();
    over.priority = 101;
    invalid.push(over);
    let mut over = input.clone();
    over.expires_at = Some("tomorrow".into());
    invalid.push(over);
    let mut over = input.clone();
    over.id = "../../elsewhere".into();
    invalid.push(over);
    let mut over = input.clone();
    over.id = uuid::Uuid::nil().to_string();
    invalid.push(over);
    for invalid in invalid {
        assert_eq!(
            repo.create_announcement(&admin, invalid).await.unwrap_err(),
            CloudError::InvalidArgument
        );
    }
    let mut value = serde_json::to_value(input).unwrap();
    value["user_id"] = serde_json::json!("other-user");
    assert!(serde_json::from_value::<CreateAnnouncement>(value).is_err());
    assert!(
        serde_json::from_value::<AcknowledgeAnnouncement>(
            serde_json::json!({"publication_revision":1,"read":true,"user_id":"other-user"})
        )
        .is_err()
    );
}
