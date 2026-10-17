use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    BrowserDocumentId, BrowserScope, BrowserSessionBinding, BrowserSessionId, BrowserTabId,
    BrowserTarget,
};
use colossus_ports::BrowserScreenshotDescriptor;
use colossus_testkit::InMemoryEventJournal;

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQMgkDAAD4AJ3MaiF4AAAAAElFTkSuQmCC";

pub(super) fn publication(owner: &str) -> BrowserArtifactPublication {
    let bytes = BASE64.decode(PNG).unwrap();
    BrowserArtifactPublication {
        binding: BrowserSessionBinding {
            runtime_id: "runtime".into(),
            workspace_id: "workspace".into(),
            application_id: owner.into(),
            scope: BrowserScope::Conversation {
                id: "conversation".into(),
            },
        },
        run_id: "run".into(),
        call_id: "call".into(),
        descriptor: BrowserScreenshotDescriptor {
            session_id: BrowserSessionId::parse(format!("bs_{}", "a".repeat(32))).unwrap(),
            target: BrowserTarget {
                tab_id: BrowserTabId::parse(format!("bt_{}", "b".repeat(32))).unwrap(),
                document_id: BrowserDocumentId::parse(format!("bd_{}", "c".repeat(32))).unwrap(),
            },
            control_generation: 1,
            transfer_id: "d".repeat(32),
            size_bytes: bytes.len() as u32,
            sha256: hex::encode(Sha256::digest(&bytes)),
            width: 1,
            height: 1,
        },
        bytes: bytes.into(),
    }
}

pub(super) fn caller(owner: &str) -> CallerContext {
    CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            owner,
            "reader",
            ApplicationKind::Embedded,
            [ApiScope::new(scopes::ARTIFACTS_READ).unwrap()],
            Vec::<String>::new(),
            Vec::<String>::new(),
        )
        .unwrap(),
        RequestId::new("read").unwrap(),
    )
}

#[tokio::test]
async fn released_png_is_idempotent_owner_bound_and_resolvable_as_output_only() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let publisher = ReleasedBrowserArtifactPublisher;
    let image = publisher
        .publish(Arc::clone(&journal), publication("app:owner"))
        .await
        .unwrap();
    let again = publisher
        .publish(Arc::clone(&journal), publication("app:owner"))
        .await
        .unwrap();
    assert_eq!(image, again);
    let service = EventSourcedArtifactApi::new(Arc::clone(&journal));
    let downloaded = service
        .download(&caller("app:owner"), &image.artifact_id, 0)
        .await
        .unwrap();
    assert_eq!(downloaded.bytes, BASE64.decode(PNG).unwrap());
    assert_eq!(downloaded.artifact.purpose, ArtifactPurpose::RunOutput);
    assert!(
        service
            .download(&caller("app:other"), &image.artifact_id, 0)
            .await
            .is_err()
    );
    let resolver = colossus_media::JournalRunInputMediaResolver::new(journal);
    assert!(
        resolver
            .image_reference("app:owner", &image.artifact_id)
            .is_err()
    );
    assert_eq!(
        resolver
            .released_image_reference("app:owner", &image.artifact_id)
            .unwrap(),
        image
    );
}

#[tokio::test]
async fn mismatched_or_non_png_release_creates_no_available_artifact() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let mut request = publication("app:owner");
    request.descriptor.sha256 = "0".repeat(64);
    assert_eq!(
        ReleasedBrowserArtifactPublisher
            .publish(Arc::clone(&journal), request)
            .await,
        Err(BrowserArtifactError::Invalid)
    );
    let mut request = publication("app:owner");
    request.bytes.clear();
    request
        .bytes
        .extend_from_slice(b"synthetic metadata cannot substitute for actual PNG bytes");
    assert_eq!(
        ReleasedBrowserArtifactPublisher
            .publish(Arc::clone(&journal), request)
            .await,
        Err(BrowserArtifactError::Invalid)
    );
    assert!(
        journal
            .list_stream_ids("artifact:", None, 100)
            .unwrap()
            .is_empty()
    );
    assert!(
        journal
            .list_stream_ids("artifact-upload:", None, 100)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn local_terminal_owner_matches_cli_artifact_interface() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let image = ReleasedBrowserArtifactPublisher
        .publish(Arc::clone(&journal), publication("terminal-user"))
        .await
        .unwrap();
    assert!(
        EventSourcedArtifactApi::new(journal)
            .get(&caller("app:colossus-cli"), &image.artifact_id)
            .await
            .is_ok()
    );
}
