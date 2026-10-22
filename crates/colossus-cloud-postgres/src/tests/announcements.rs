//! Real native-column, audit, restart and multi-replica announcement persistence.
use super::*;
use colossus_cloud::{
    CloudRepository, OidcIdentity,
    announcements::{
        AcknowledgeAnnouncement, AnnouncementTone, CreateAnnouncement, CreateAnnouncementAction,
        UpdateAnnouncement, UpdateAnnouncementAction,
    },
};
use diesel::{sql_query, sql_types::Text};
use diesel_async::RunQueryDsl;
use std::sync::Arc;

fn input() -> CreateAnnouncement {
    CreateAnnouncement {
        id: uuid::Uuid::now_v7().to_string(),
        title: "Published feature".into(),
        summary: "A small banner".into(),
        body: "# Feature\n\n```rust\nfn main() {}\n```\n".into(),
        tone: AnnouncementTone::Success,
        banner: true,
        priority: 70,
        expires_at: None,
        action: CreateAnnouncementAction::Publish,
    }
}

#[tokio::test]
#[ignore = "requires an explicit isolated local PostgreSQL fixture"]
async fn postgres_announcements_reopen_replica_receipts_native_columns_and_audit() {
    let config = config();
    let store = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .unwrap(),
    );
    let repo = CloudRepository::new(store.clone()).unwrap();
    for (id, admin) in [
        ("ann-admin", true),
        ("ann-alice", false),
        ("ann-bob", false),
    ] {
        repo.create_account(
            "fixture",
            super::identity::account(id, admin),
            Some(OidcIdentity {
                user_id: id.into(),
                issuer: "https://issuer.example.test".into(),
                subject: id.into(),
            }),
            None,
        )
        .await
        .unwrap();
    }
    let admin = repo.account("ann-admin").await.unwrap().user;
    let alice = repo.account("ann-alice").await.unwrap().user;
    let bob = repo.account("ann-bob").await.unwrap().user;
    let initial = input();
    let published = repo
        .create_announcement(&admin, initial.clone())
        .await
        .unwrap();
    assert_eq!(
        repo.create_announcement(&admin, initial).await.unwrap(),
        published
    );
    let replica_store = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .unwrap(),
    );
    let replica = CloudRepository::new(replica_store.clone()).unwrap();
    assert_eq!(
        replica
            .admin_announcement(&admin, &published.id)
            .await
            .unwrap(),
        published
    );
    let (read, dismiss) = tokio::join!(
        repo.acknowledge_announcement(
            &alice,
            &published.id,
            AcknowledgeAnnouncement {
                publication_revision: 1,
                read: Some(true),
                dismissed: None,
            }
        ),
        replica.acknowledge_announcement(
            &alice,
            &published.id,
            AcknowledgeAnnouncement {
                publication_revision: 1,
                read: None,
                dismissed: Some(true),
            }
        )
    );
    assert!(read.unwrap().read);
    assert!(dismiss.unwrap().dismissed);
    let seen = replica.announcements(&alice, None, 20).await.unwrap();
    assert!(seen.announcements[0].read && seen.announcements[0].dismissed);
    let unseen = repo.announcements(&bob, None, 20).await.unwrap();
    assert!(!unseen.announcements[0].read && !unseen.announcements[0].dismissed);

    // Filtering happens on native columns before the page limit. Later expired
    // or unpublished records cannot mask the older active item.
    let mut expired = input();
    expired.expires_at = Some("2000-01-01T00:00:00Z".into());
    repo.create_announcement(&admin, expired).await.unwrap();
    let mut draft = input();
    draft.action = CreateAnnouncementAction::Draft;
    repo.create_announcement(&admin, draft).await.unwrap();
    assert_eq!(
        replica
            .announcements(&bob, None, 1)
            .await
            .unwrap()
            .announcements[0]
            .announcement
            .id,
        published.id
    );

    let changed = replica
        .update_announcement(
            &admin,
            &published.id,
            UpdateAnnouncement {
                revision: published.revision,
                title: published.title.clone(),
                summary: published.summary.clone(),
                body: "A second content revision".into(),
                tone: published.tone,
                banner: true,
                priority: 70,
                expires_at: None,
                action: UpdateAnnouncementAction::Publish,
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.publication_revision, 2);
    assert!(
        !repo
            .announcements(&alice, None, 20)
            .await
            .unwrap()
            .announcements[0]
            .read
    );

    // Native selection must find an undismissed banner independently of both
    // center-only records and already-dismissed banners beyond the first page.
    for _ in 0..25 {
        let mut center_only = input();
        center_only.banner = false;
        center_only.priority = 100;
        repo.create_announcement(&admin, center_only).await.unwrap();
        let mut banner = input();
        banner.priority = 100;
        let banner = repo.create_announcement(&admin, banner).await.unwrap();
        replica
            .acknowledge_announcement(
                &alice,
                &banner.id,
                AcknowledgeAnnouncement {
                    publication_revision: 1,
                    read: None,
                    dismissed: Some(true),
                },
            )
            .await
            .unwrap();
    }
    let page = repo.announcements(&alice, None, 20).await.unwrap();
    assert_eq!(page.announcements.len(), 20);
    assert!(
        page.announcements
            .iter()
            .all(|entry| entry.announcement.id != published.id)
    );
    assert_eq!(
        page.active_banner.as_ref().unwrap().announcement.id,
        published.id
    );
    let mut after = page.next_cursor;
    let mut identities = page
        .announcements
        .into_iter()
        .map(|entry| entry.announcement.id)
        .collect::<std::collections::BTreeSet<_>>();
    while let Some(cursor) = after {
        let page = replica
            .announcements(&alice, Some(&cursor), 20)
            .await
            .unwrap();
        for entry in page.announcements {
            assert!(identities.insert(entry.announcement.id));
        }
        after = page.next_cursor;
    }
    assert_eq!(identities.len(), 51);
    let promoted_input = UpdateAnnouncement {
        revision: changed.revision,
        title: changed.title.clone(),
        summary: changed.summary.clone(),
        body: "An elevated new publication from the oldest record".into(),
        tone: changed.tone,
        banner: true,
        priority: 100,
        expires_at: None,
        action: UpdateAnnouncementAction::Publish,
    };
    let (left, right) = tokio::join!(
        repo.update_announcement(&admin, &published.id, promoted_input.clone()),
        replica.update_announcement(&admin, &published.id, promoted_input),
    );
    let promoted = left.unwrap();
    assert_eq!(right.unwrap(), promoted);
    assert_eq!(promoted.revision, changed.revision + 1);
    assert_eq!(promoted.publication_revision, 3);
    let reordered = replica.announcements(&alice, None, 20).await.unwrap();
    assert_eq!(reordered.announcements[0].announcement.id, published.id);
    assert_eq!(
        reordered.active_banner.as_ref().unwrap().announcement.id,
        published.id
    );

    #[derive(diesel::QueryableByName)]
    struct Native {
        #[diesel(sql_type = Text)]
        body: String,
        #[diesel(sql_type = Text)]
        publication_status: String,
    }
    let mut connection = store.pool.get().await.unwrap();
    let native = sql_query("SELECT body,publication_status FROM announcements WHERE id=$1")
        .bind::<Text, _>(&published.id)
        .get_result::<Native>(&mut connection)
        .await
        .unwrap();
    assert_eq!(native.body, promoted.body);
    assert_eq!(native.publication_status, "published");
    sql_query("UPDATE announcements SET body='tampered body' WHERE id=$1")
        .bind::<Text, _>(&published.id)
        .execute(&mut connection)
        .await
        .unwrap();
    drop(connection);
    assert_eq!(
        replica
            .admin_announcement(&admin, &published.id)
            .await
            .unwrap_err(),
        CloudError::Storage
    );
    store.remove_fixture_schema(&config.schema).await.unwrap();
}
