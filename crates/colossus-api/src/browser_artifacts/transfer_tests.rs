use super::tests::{caller, publication};
use super::*;
use colossus_ports::{
    BrowserDownloadDescriptor, BrowserDownloadPublication, BrowserUploadArtifactRequest,
};

async fn input(
    journal: Arc<dyn EventJournal>,
    owner: &str,
    purpose: ArtifactPurpose,
    name: &str,
    bytes: &[u8],
) -> String {
    let principal = ApplicationPrincipal::authenticated(
        owner,
        "transfer-test",
        ApplicationKind::Embedded,
        [ApiScope::new(scopes::ARTIFACTS_WRITE).unwrap()],
        Vec::<String>::new(),
        Vec::<String>::new(),
    )
    .unwrap();
    let caller = CallerContext::authenticated(principal, RequestId::new("create-input").unwrap());
    let service = EventSourcedArtifactApi::new(journal);
    let upload = service
        .create_upload(
            &caller,
            CreateArtifactUploadRequest {
                file_name: name.into(),
                media_type: "application/octet-stream".into(),
                size_bytes: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
                purpose,
                idempotency_key: IdempotencyKey::new(name).unwrap(),
            },
        )
        .await
        .unwrap();
    service
        .upload(
            &caller,
            &upload.upload_id,
            vec![ArtifactChunk {
                offset: 0,
                data: bytes.to_vec(),
            }],
        )
        .await
        .unwrap()
        .artifact_id
}

fn request(owner: &str, artifact_id: String) -> BrowserUploadArtifactRequest {
    let sample = publication(owner);
    BrowserUploadArtifactRequest {
        binding: sample.binding,
        run_id: sample.run_id,
        artifact_id,
    }
}

#[tokio::test]
async fn upload_custody_requires_the_exact_owner_and_input_or_output_purpose() {
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    let id = input(
        Arc::clone(&journal),
        "app:owner",
        ArtifactPurpose::RunInput,
        "ordinary.txt",
        b"owned bytes",
    )
    .await;
    let publisher = ReleasedBrowserArtifactPublisher;
    let resolved = publisher
        .resolve_upload(Arc::clone(&journal), request("app:owner", id.clone()))
        .await
        .unwrap();
    assert_eq!(&*resolved.bytes, b"owned bytes");
    assert_eq!(
        resolved.descriptor.sha256,
        hex::encode(Sha256::digest(b"owned bytes"))
    );
    assert!(
        publisher
            .resolve_upload(Arc::clone(&journal), request("app:foreign", id))
            .await
            .is_err()
    );
    let output = input(
        Arc::clone(&journal),
        "app:owner",
        ArtifactPurpose::RunOutput,
        "output.txt",
        b"released output",
    )
    .await;
    let resolved = publisher
        .resolve_upload(Arc::clone(&journal), request("app:owner", output))
        .await
        .unwrap();
    assert_eq!(&*resolved.bytes, b"released output");
    for (purpose, name) in [
        (ArtifactPurpose::Workflow, "workflow.json"),
        (ArtifactPurpose::Extension, "extension.zip"),
        (ArtifactPurpose::Archive, "archive.zip"),
    ] {
        let id = input(
            Arc::clone(&journal),
            "app:owner",
            purpose,
            name,
            b"other artifact purpose",
        )
        .await;
        assert!(
            publisher
                .resolve_upload(Arc::clone(&journal), request("app:owner", id))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn native_key_entry_formats_and_private_key_content_are_not_browser_uploads() {
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    let publisher = ReleasedBrowserArtifactPublisher;
    for (name, bytes) in [
        ("identity.pfx", b"binary fixture".as_slice()),
        ("ordinary.txt", b"-----BEGIN PRIVATE KEY-----\nfixture"),
    ] {
        let id = input(
            Arc::clone(&journal),
            "app:owner",
            ArtifactPurpose::RunInput,
            name,
            bytes,
        )
        .await;
        assert!(
            publisher
                .resolve_upload(Arc::clone(&journal), request("app:owner", id))
                .await
                .is_err()
        );
    }
}

fn downloaded(bytes: &[u8]) -> BrowserDownloadPublication {
    let sample = publication("app:owner");
    BrowserDownloadPublication {
        binding: sample.binding,
        run_id: sample.run_id,
        call_id: sample.call_id,
        descriptor: BrowserDownloadDescriptor {
            session_id: sample.descriptor.session_id,
            target: sample.descriptor.target,
            control_generation: 1,
            transfer_id: "e".repeat(32),
            size_bytes: bytes.len() as u32,
            sha256: hex::encode(Sha256::digest(bytes)),
            origin: colossus_contracts::BrowserOrigin::parse("https://example.test").unwrap(),
        },
        bytes: bytes.to_vec().into(),
    }
}

#[tokio::test]
async fn released_download_is_idempotent_binary_output_and_owner_bound() {
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    let publisher = ReleasedBrowserArtifactPublisher;
    let bytes = b"actual complete downloaded bytes\0";
    let artifact = publisher
        .publish_download(Arc::clone(&journal), downloaded(bytes))
        .await
        .unwrap();
    assert_eq!(
        artifact,
        publisher
            .publish_download(Arc::clone(&journal), downloaded(bytes))
            .await
            .unwrap()
    );
    assert_eq!(artifact.file_name, "browser-download.bin");
    let upload = publisher
        .resolve_upload(
            Arc::clone(&journal),
            request("app:owner", artifact.artifact_id.clone()),
        )
        .await
        .unwrap();
    assert_eq!(&*upload.bytes, bytes);
    assert_eq!(upload.descriptor.artifact_id, artifact.artifact_id);
    assert_eq!(upload.descriptor.file_name, "upload.bin");
    assert_eq!(upload.descriptor.size_bytes, bytes.len() as u32);
    assert_eq!(upload.descriptor.sha256, artifact.sha256);
    assert!(
        publisher
            .resolve_upload(
                Arc::clone(&journal),
                request("app:foreign", artifact.artifact_id.clone()),
            )
            .await
            .is_err()
    );
    let service = EventSourcedArtifactApi::new(journal);
    let resolved = service
        .download(&caller("app:owner"), &artifact.artifact_id, 0)
        .await
        .unwrap();
    assert_eq!(resolved.artifact.purpose, ArtifactPurpose::RunOutput);
    assert_eq!(resolved.bytes, bytes);
    assert!(
        service
            .download(&caller("app:foreign"), &artifact.artifact_id, 0)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn invalid_actual_download_bytes_never_create_an_available_artifact() {
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    let mut invalid = downloaded(b"actual bytes");
    invalid.descriptor.sha256 = "0".repeat(64);
    assert!(
        ReleasedBrowserArtifactPublisher
            .publish_download(Arc::clone(&journal), invalid)
            .await
            .is_err()
    );
    assert!(
        journal
            .list_stream_ids("artifact:", None, 100)
            .unwrap()
            .is_empty()
    );
}
