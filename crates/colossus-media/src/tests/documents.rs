use super::*;
use colossus_contracts::ModelFileReference;
use std::io::Write as _;

fn office(parts: &[&str]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for name in parts {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer
            .write_all(b"<synthetic>bounded fixture</synthetic>")
            .unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn native_documents_check_filename_mime_and_bounded_container_without_extracting() {
    let pdf = b"%PDF-1.7\nsynthetic envelope\n%%EOF\n";
    let validated = validate_document_bytes("report.pdf", Some("application/pdf"), pdf).unwrap();
    assert_eq!(validated.sha256, hex::encode(Sha256::digest(pdf)));
    for (name, part) in [
        ("report.docx", "word/document.xml"),
        ("book.xlsx", "xl/workbook.xml"),
        ("slides.pptx", "ppt/presentation.xml"),
    ] {
        let bytes = office(&["[Content_Types].xml", part]);
        let valid = validate_document_bytes(name, None, &bytes).unwrap();
        assert_eq!(
            valid.media_type,
            document_media_type_for_file_name(name).unwrap()
        );
        assert_eq!(valid.size_bytes, bytes.len() as u64);
        assert!(validate_document_bytes("wrong.pdf", None, &bytes).is_err());
        assert!(validate_document_bytes(name, Some("application/pdf"), &bytes).is_err());
        assert!(validate_document_bytes(name, None, pdf).is_err());
    }
    for name in [
        "../report.pdf",
        "folder/report.pdf",
        "folder\\report.pdf",
        " report.pdf",
        "report.pdf\0",
    ] {
        assert!(validate_document_bytes(name, None, pdf).is_err());
    }
    assert!(validate_document_bytes("report.pdf", None, b"plain text").is_err());
    assert!(validate_document_bytes("report.pdf", None, b"%PDF-1.7\nmissing trailer").is_err());
    assert!(validate_document_bytes("report.docm", None, pdf).is_err());
}

#[test]
fn compressed_office_metadata_is_admitted_without_enabling_decompression() {
    // A deterministic Python zipfile ZIP_DEFLATED envelope. The default-feature-free
    // ZIP reader must inspect common Office packages without extracting XML content.
    let bytes = BASE64
        .decode(concat!(
            "UEsDBBQAAAAIAAAAIVxq6kWfMQAAADcAAAATAAAAW0NvbnRlbnRfVHlwZXNdLnhtbLMprswryUgtyUy2",
            "S87PLShKLS5OTVHITS1JTEksSdTNz8upVEjLrCgpLUq10UeoBQBQSwMEFAAAAAgAAAAhXGrqRZ8x",
            "AAAANwAAABEAAAB3b3JkL2RvY3VtZW50LnhtbLMprswryUgtyUy2S87PLShKLS5OTVHITS1JTEks",
            "SdTNz8upVEjLrCgpLUq10UeoBQBQSwECFAAUAAAACAAAACFcaupFnzEAAAA3AAAAEwAAAAAAAAAAAAAA",
            "gAEAAAAAW0NvbnRlbnRfVHlwZXNdLnhtbFBLAQIUABQAAAAIAAAAIVxq6kWfMQAAADcAAAARAAAAAAAA",
            "AAAAAACAAWIAAAB3b3JkL2RvY3VtZW50LnhtbFBLBQYAAAAAAgACAIAAAADCAAAAAAA="
        ))
        .unwrap();
    let valid = validate_document_bytes("compressed.docx", None, &bytes).unwrap();
    assert_eq!(valid.size_bytes, bytes.len() as u64);
    assert_eq!(valid.sha256, hex::encode(Sha256::digest(&bytes)));
}

#[test]
fn office_inputs_reject_arbitrary_zip_mismatched_parts_macros_and_unsafe_entries() {
    for parts in [
        vec!["[Content_Types].xml", "xl/workbook.xml"],
        vec![
            "[Content_Types].xml",
            "word/document.xml",
            "word/vbaProject.bin",
        ],
        vec!["[Content_Types].xml", "word/document.xml", "../escape"],
        vec!["word/document.xml"],
    ] {
        assert!(validate_document_bytes("report.docx", None, &office(&parts)).is_err());
    }
    let mut bytes = office(&["[Content_Types].xml", "word/document.xml"]);
    let tail = bytes.windows(4).rposition(|s| s == b"PK\x05\x06").unwrap();
    bytes[tail + 10..tail + 12].copy_from_slice(&2049u16.to_le_bytes());
    assert!(validate_document_bytes("report.docx", None, &bytes).is_err());
}

fn file_reference(size: u64) -> ModelFileReference {
    ModelFileReference {
        artifact_id: format!("artifact-{}", "1".repeat(64)),
        file_name: "report.pdf".into(),
        media_type: "application/pdf".into(),
        size_bytes: size,
        sha256: "a".repeat(64),
    }
}

#[test]
fn documents_have_independent_count_per_file_and_combined_byte_bounds() {
    let small = file_reference(1);
    validate_document_references([&small; 4]).unwrap();
    assert!(validate_document_references([&small; 5]).is_err());
    let zero = file_reference(0);
    assert!(validate_document_references([&zero]).is_err());
    let large = file_reference(MAX_DOCUMENT_BYTES);
    validate_document_references([&large; 2]).unwrap();
    assert!(validate_document_references([&large; 3]).is_err());
    assert!(validate_document_references([&file_reference(MAX_DOCUMENT_BYTES + 1)]).is_err());
}

fn store_document(journal: &dyn EventJournal, purpose: &str, forged_digest: bool) -> String {
    let id = format!("artifact-{}", "1".repeat(64));
    let bytes = b"%PDF-1.7\nprivate synthetic payload\n%%EOF\n";
    journal.append(NewEvent {
        event_version: 1, stream_id: format!("artifact:{id}"), expected_stream_version: 0,
        event_type: AVAILABLE_EVENT.into(), classification: EventClassification::System,
        actor: Actor { actor_type: ActorType::Application, id: "app:document-owner".into() },
        context: ExecutionContext::default(),
        payload: json!({"artifact": {"artifact_id":id,"file_name":"report.pdf","media_type":"application/pdf",
            "size_bytes":bytes.len(),"sha256":if forged_digest { "0".repeat(64) } else { hex::encode(Sha256::digest(bytes)) },
            "purpose":purpose,"state":"available","created_at":"2026-11-01T00:00:00Z"},
            "content_base64":BASE64.encode(bytes)}),
    }).unwrap();
    id
}

#[tokio::test]
async fn document_admission_requires_owner_and_run_input_and_resolution_rechecks_exact_metadata() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let id = store_document(journal.as_ref(), "run_input", false);
    let resolver = JournalRunInputMediaResolver::new(journal);
    assert!(resolver.file_reference("app:other", &id).is_err());
    let reference = resolver.file_reference("app:document-owner", &id).unwrap();
    let resolved = resolver.resolve_file(&reference).await.unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&resolved.bytes)),
        reference.sha256
    );
    assert!(
        !serde_json::to_string(&reference)
            .unwrap()
            .contains("private synthetic payload")
    );
    for mutated in [
        ModelFileReference {
            sha256: "0".repeat(64),
            ..reference.clone()
        },
        ModelFileReference {
            size_bytes: 1,
            ..reference.clone()
        },
        ModelFileReference {
            file_name: "renamed.pdf".into(),
            ..reference.clone()
        },
    ] {
        assert!(resolver.resolve_file(&mutated).await.is_err());
    }
    for (purpose, forged) in [("run_output", false), ("run_input", true)] {
        let journal = Arc::new(InMemoryEventJournal::default());
        let id = store_document(journal.as_ref(), purpose, forged);
        assert!(
            JournalRunInputMediaResolver::new(journal)
                .file_reference("app:document-owner", &id)
                .is_err()
        );
    }
}
