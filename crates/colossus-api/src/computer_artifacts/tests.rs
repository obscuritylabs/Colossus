use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    ComputerBinding, ComputerObservationId, ComputerScreenshotDescriptor, ComputerSessionId,
    ComputerWindowId,
};
use colossus_testkit::InMemoryEventJournal;

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQMgkDAAD4AJ3MaiF4AAAAAElFTkSuQmCC";

fn publication(owner: &str) -> ComputerArtifactPublication {
    let bytes = BASE64.decode(PNG).unwrap();
    ComputerArtifactPublication {
        binding: ComputerBinding {
            runtime_id: "runtime".into(),
            workspace_id: "workspace".into(),
            application_id: owner.into(),
            scope: ComputerScope::Conversation {
                id: "conversation".into(),
            },
            run_id: "run".into(),
        },
        call_id: "call".into(),
        descriptor: ComputerScreenshotDescriptor {
            session_id: ComputerSessionId::parse(format!("cu_s_{}", "a".repeat(32))).unwrap(),
            control_generation: 1,
            observation_id: ComputerObservationId::parse(format!("cu_o_{}", "b".repeat(32)))
                .unwrap(),
            window_id: ComputerWindowId::parse(format!("cu_w_{}", "c".repeat(32))).unwrap(),
            width: 1,
            height: 1,
            size_bytes: bytes.len() as u32,
            sha256: hex::encode(Sha256::digest(&bytes)),
        },
        bytes: bytes.into(),
    }
}

fn caller(owner: &str) -> CallerContext {
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
async fn window_png_is_idempotent_owned_run_output_and_cannot_be_imported_as_input() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let publisher = ReleasedComputerArtifactPublisher;
    let image = publisher
        .publish(Arc::clone(&journal), publication("app:owner"))
        .await
        .unwrap();
    assert_eq!(
        image,
        publisher
            .publish(Arc::clone(&journal), publication("app:owner"))
            .await
            .unwrap()
    );
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
async fn invalid_capture_or_unproved_workflow_owner_publishes_nothing() {
    for violation in 0..5 {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let mut request = publication("app:owner");
        match violation {
            0 => request.descriptor.sha256 = "0".repeat(64),
            1 => request.descriptor.width = 2,
            2 => request.descriptor.control_generation = 0,
            3 => {
                request.bytes.clear();
                request.bytes.extend_from_slice(b"not an image");
            }
            _ => {
                request.binding.scope = ComputerScope::Workflow {
                    id: "app:owner".into(),
                }
            }
        }
        assert_eq!(
            ReleasedComputerArtifactPublisher
                .publish(Arc::clone(&journal), request)
                .await,
            Err(ComputerArtifactError::Invalid)
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
}

#[tokio::test]
async fn distinct_runs_do_not_alias_and_cli_owner_remains_readable() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let image = ReleasedComputerArtifactPublisher
        .publish(Arc::clone(&journal), publication("terminal-user"))
        .await
        .unwrap();
    let mut request = publication("terminal-user");
    request.binding.run_id = "next-run".into();
    let next = ReleasedComputerArtifactPublisher
        .publish(Arc::clone(&journal), request)
        .await
        .unwrap();
    assert_ne!(image.artifact_id, next.artifact_id);
    assert!(
        EventSourcedArtifactApi::new(journal)
            .get(&caller("app:colossus-cli"), &image.artifact_id)
            .await
            .is_ok()
    );
}
