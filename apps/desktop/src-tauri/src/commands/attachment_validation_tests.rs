use super::*;

fn model(profile: &str, roles: &[&str], media_types: &[&str]) -> ConfiguredModelProfile {
    ConfiguredModelProfile {
        profile: profile.into(),
        model: "synthetic-model".into(),
        provider: "openai_responses".into(),
        provider_profile: "synthetic-provider".into(),
        reasoning_effort: None,
        default_roles: roles.iter().map(|role| (*role).into()).collect(),
        document_media_types: media_types.iter().map(|value| (*value).into()).collect(),
    }
}

#[test]
fn document_admission_uses_the_exact_authenticated_model_choice() {
    let profiles = vec![
        model("main", &["primary"], &["application/pdf"]),
        model("review", &["reviewer"], &[]),
    ];
    assert!(require_document_support(&profiles, "primary", None, "application/pdf").is_ok());
    assert!(
        require_document_support(&profiles, "unconfigured-role", None, "application/pdf").is_ok()
    );
    assert!(require_document_support(&profiles, "reviewer", None, "application/pdf").is_err());
    assert!(
        require_document_support(&profiles, "primary", Some("review"), "application/pdf").is_err()
    );
    assert!(
        require_document_support(&profiles, "reviewer", Some("main"), "application/pdf").is_ok()
    );
    assert!(
        require_document_support(&profiles, "primary", Some("removed"), "application/pdf").is_err()
    );
    assert!(require_document_support(&profiles, "", None, "application/pdf").is_err());
    assert!(require_document_support(&[], "primary", None, "application/pdf").is_err());
}

#[test]
fn pdf_support_does_not_admit_office_or_unknown_formats() {
    let profiles = vec![model(
        "main",
        &["primary"],
        &["application/pdf", "application/octet-stream"],
    )];
    assert!(
        require_document_support(
            &profiles,
            "primary",
            None,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        )
        .is_err()
    );
    assert!(
        require_document_support(&profiles, "primary", None, "application/octet-stream").is_err()
    );
}

#[test]
fn documents_and_text_follow_distinct_byte_validation_paths() {
    assert!(validate_attachment_bytes("notes.csv", "text/csv", b"item,count\napples,3\n").is_ok());
    assert!(
        validate_attachment_bytes("notes.ps1", "text/plain", b"Write-Output 'hello'\n").is_ok()
    );
    assert!(validate_attachment_bytes("notes.csv", "text/csv", &[0xff, 0xfe]).is_err());
    assert!(
        validate_attachment_bytes(
            "report.pdf",
            "application/pdf",
            b"plain text with a PDF suffix"
        )
        .is_err()
    );
    assert!(
        validate_attachment_bytes(
            "report.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            b"plain text with a DOCX suffix",
        )
        .is_err()
    );
}

#[test]
fn text_format_expansion_keeps_unknown_binaries_rejected() {
    for name in [
        "report.csv",
        "REPORT.TSV",
        "application.log",
        "configure.ps1",
        "app.conf",
        "Dockerfile",
        "Makefile",
    ] {
        assert!(attachment_media_type(Path::new(name)).is_some(), "{name}");
    }
    for name in [
        "program.exe",
        "archive.zip",
        "macro.docm",
        "macro.xlsm",
        "legacy.doc",
    ] {
        assert!(attachment_media_type(Path::new(name)).is_none(), "{name}");
    }
}
