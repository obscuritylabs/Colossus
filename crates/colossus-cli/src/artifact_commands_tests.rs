use super::{cli_artifact_caller, prepare_model_content};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_api::{ArtifactApi as _, EventSourcedArtifactApi};
use colossus_contracts::{ModelContent, ModelContentPart};
use colossus_ports::{CredentialResolutionError, CredentialResolver};
use colossus_runtime::Runtime;
use sha2::{Digest as _, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const PDF: &[u8] = b"%PDF-1.7\nsynthetic adapter fixture\n%%EOF\n";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQMgkDAAD4AJ3MaiF4AAAAAElFTkSuQmCC";

struct NoCredentials;

impl CredentialResolver for NoCredentials {
    fn resolve(&self, _: &str) -> Result<String, CredentialResolutionError> {
        panic!("attachment assembly must not resolve provider credentials")
    }
}

fn fixture() -> (tempfile::TempDir, Runtime) {
    let directory = tempfile::tempdir().expect("attachment fixture");
    let root = directory.path().canonicalize().expect("canonical fixture");
    let mut config = colossus_runtime::RuntimeConfig::offline_template(root.join("unused.redb"));
    config.use_ephemeral_storage();
    config.workflows.repository = root.join("workflows-bundled");
    config.workflows.user = root.join("workflows-user");
    for path in [&config.workflows.repository, &config.workflows.user] {
        std::fs::create_dir_all(path).expect("fixture workflows");
    }
    config
        .sandbox
        .filesystem
        .push(colossus_contracts::FilesystemGrant {
            root: root.display().to_string(),
            mode: "read".into(),
        });
    let runtime = Runtime::open_with_provider_credentials(
        &config,
        Arc::new(colossus_policy::DenyApproval),
        None,
        colossus_runtime::RuntimeOpenOptions::for_workspace(&root).expect("fixture workspace"),
        Arc::new(NoCredentials),
    )
    .expect("offline attachment runtime");
    (directory, runtime)
}

fn write(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, bytes).expect("write attachment");
    path
}

#[tokio::test]
async fn mixed_attachments_preserve_input_order_and_exact_document_bytes() {
    let (directory, runtime) = fixture();
    let png = BASE64.decode(PNG).expect("PNG fixture");
    let paths = [
        write(directory.path(), "before.txt", b"first text marker"),
        write(directory.path(), "proposal.pdf", PDF),
        write(directory.path(), "diagram.png", &png),
        write(directory.path(), "after.csv", b"final,text\nmarker,1\n"),
    ];
    let content = prepare_model_content(&runtime, "Review in this order", &paths)
        .await
        .expect("mixed attachment content");
    let ModelContent::Parts(parts) = content else {
        panic!("mixed native media must remain ordered multipart content");
    };
    assert_eq!(parts.len(), 5);
    assert!(matches!(&parts[0], ModelContentPart::Text { text } if text == "Review in this order"));
    assert!(
        matches!(&parts[1], ModelContentPart::Text { text } if text.contains("first text marker"))
    );
    let ModelContentPart::File { file } = &parts[2] else {
        panic!("the PDF must remain between the first text and image");
    };
    assert_eq!(file.file_name, "proposal.pdf");
    assert_eq!(file.media_type, "application/pdf");
    assert_eq!(file.size_bytes, u64::try_from(PDF.len()).unwrap());
    assert_eq!(file.sha256, hex::encode(Sha256::digest(PDF)));
    let ModelContentPart::Image { image } = &parts[3] else {
        panic!("the image must remain after the PDF");
    };
    assert_eq!(image.file_name, "diagram.png");
    assert!(matches!(&parts[4], ModelContentPart::Text { text } if text.contains("final,text")));
    let downloaded = EventSourcedArtifactApi::new(runtime.journal())
        .download(
            &cli_artifact_caller().expect("caller"),
            &file.artifact_id,
            0,
        )
        .await
        .expect("owner-authorized document download");
    assert_eq!(downloaded.bytes, PDF);
    assert_eq!(
        runtime
            .run_input_file_reference("app:colossus-cli", &file.artifact_id)
            .expect("canonical document reference"),
        *file
    );
    assert!(
        runtime
            .run_input_file_reference("app:other", &file.artifact_id)
            .is_err()
    );
}

#[tokio::test]
async fn forged_documents_and_images_do_not_fall_back_to_text() {
    let (directory, runtime) = fixture();
    for (name, bytes) in [
        ("forged.pdf", b"valid UTF-8 is not a PDF".as_slice()),
        ("forged.docx", b"valid UTF-8 is not Office".as_slice()),
        ("forged.png", b"valid UTF-8 is not an image".as_slice()),
        ("unknown.bin", [0xff, 0xfe].as_slice()),
    ] {
        let path = write(directory.path(), name, bytes);
        assert!(
            prepare_model_content(&runtime, "Review", &[path])
                .await
                .is_err(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn document_count_and_input_preflight_bounds_are_enforced() {
    let (directory, runtime) = fixture();
    let paths = (0..5)
        .map(|index| write(directory.path(), &format!("report-{index}.pdf"), PDF))
        .collect::<Vec<_>>();
    let ModelContent::Parts(parts) = prepare_model_content(&runtime, "", &paths[..4])
        .await
        .expect("four bounded documents")
    else {
        panic!("documents must remain multipart");
    };
    assert_eq!(parts.len(), 5);
    assert_eq!(
        parts
            .iter()
            .filter(|part| matches!(part, ModelContentPart::File { .. }))
            .count(),
        4
    );
    let error = prepare_model_content(&runtime, "", &paths)
        .await
        .expect_err("fifth document");
    assert!(error.to_string().contains("4-file"), "{error}");

    let before = runtime.journal().head().expect("journal head");
    let missing = directory.path().join("must-not-be-read.pdf");
    let error = prepare_model_content(&runtime, "", &vec![missing.clone(); 17])
        .await
        .expect_err("attachment count preflight");
    assert!(
        error.to_string().contains("at most 16 attachments"),
        "{error}"
    );
    let error = prepare_model_content(&runtime, &"x".repeat(1_048_577), &[missing])
        .await
        .expect_err("prompt size preflight");
    assert!(error.to_string().contains("exceed 1 MiB"), "{error}");
    assert_eq!(
        runtime.journal().head().expect("journal after preflight"),
        before,
        "preflight rejection must not read or import an attachment"
    );
}

#[tokio::test]
async fn combined_document_bytes_and_individual_read_limits_are_enforced() {
    let (directory, runtime) = fixture();
    let mut bytes = vec![b' '; 11 * 1_048_576];
    bytes[..8].copy_from_slice(b"%PDF-1.7");
    let tail = bytes.len() - 6;
    bytes[tail..].copy_from_slice(b"%%EOF\n");
    let paths = (0..3)
        .map(|index| write(directory.path(), &format!("large-{index}.pdf"), &bytes))
        .collect::<Vec<_>>();
    let error = prepare_model_content(&runtime, "", &paths)
        .await
        .expect_err("combined document bound");
    assert!(error.to_string().contains("32 MiB"), "{error}");
    let mut oversized = vec![b' '; 16 * 1_048_576 + 1];
    oversized[..8].copy_from_slice(b"%PDF-1.7");
    let path = write(directory.path(), "oversized.pdf", &oversized);
    assert!(prepare_model_content(&runtime, "", &[path]).await.is_err());
}
