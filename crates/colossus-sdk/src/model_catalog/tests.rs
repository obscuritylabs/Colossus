use super::*;
use colossus_api_proto::v1alpha1::ConfiguredModelProfile as WireProfile;

fn profile() -> WireProfile {
    WireProfile {
        profile: "primary-model".into(),
        model: "provider-model".into(),
        provider: "responses".into(),
        provider_profile: "work".into(),
        reasoning_effort: Some("ultra".into()),
        default_roles: vec!["primary".into()],
        document_media_types: Vec::new(),
    }
}
fn supported() -> crate::ServerCapabilities {
    crate::ServerCapabilities::from_enabled(["agent_runs.model_selection".into()])
}

#[test]
fn configured_catalog_preserves_authenticated_display_and_route_metadata() {
    let decoded = decode(vec![profile()], &supported()).unwrap();
    assert_eq!(decoded[0].profile, "primary-model");
    assert_eq!(decoded[0].provider_profile, "work");
    assert_eq!(decoded[0].reasoning_effort.as_deref(), Some("ultra"));
    assert_eq!(decoded[0].default_roles, ["primary"]);
}

#[test]
fn configured_catalog_is_empty_when_unsupported_or_omitted() {
    assert!(decode(Vec::new(), &supported()).unwrap().is_empty());
    assert!(
        decode(vec![profile()], &crate::ServerCapabilities::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn configured_catalog_rejects_unbounded_or_ambiguous_metadata() {
    assert!(decode(vec![profile(); 257], &supported()).is_err());
    assert!(decode(vec![profile(), profile()], &supported()).is_err());
    assert!(
        decode(
            vec![
                profile(),
                WireProfile {
                    profile: "other".into(),
                    ..profile()
                }
            ],
            &supported()
        )
        .is_err()
    );
    for invalid in [
        WireProfile {
            profile: String::new(),
            ..profile()
        },
        WireProfile {
            model: "unsafe\nlabel".into(),
            ..profile()
        },
        WireProfile {
            provider_profile: "p".repeat(513),
            ..profile()
        },
        WireProfile {
            default_roles: vec!["primary".into(); 65],
            ..profile()
        },
        WireProfile {
            reasoning_effort: Some("unknown".into()),
            ..profile()
        },
    ] {
        assert!(decode(vec![invalid], &supported()).is_err());
    }
}

#[test]
fn document_choices_require_the_authenticated_document_capability() {
    let mut wire = profile();
    wire.document_media_types = vec![
        "application/pdf".into(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation".into(),
    ];
    let capabilities = crate::ServerCapabilities::from_enabled([
        "agent_runs.model_selection".into(),
        "attachments.document_input".into(),
    ]);
    let decoded = decode(vec![wire.clone()], &capabilities).unwrap();
    assert_eq!(decoded[0].document_media_types, wire.document_media_types);
    assert!(
        decode(vec![wire], &supported()).unwrap()[0]
            .document_media_types
            .is_empty()
    );
    assert!(
        decode(vec![profile()], &capabilities).unwrap()[0]
            .document_media_types
            .is_empty()
    );
}

#[test]
fn document_choices_reject_unknown_duplicate_or_unbounded_media_types() {
    let capabilities = crate::ServerCapabilities::from_enabled([
        "agent_runs.model_selection".into(),
        "attachments.document_input".into(),
    ]);
    for media_types in [
        vec!["application/pdf"; 5],
        vec!["application/pdf", "application/pdf"],
        vec!["application/zip"],
        vec!["application/pdf\n"],
        vec!["APPLICATION/PDF"],
        vec![""],
    ] {
        let mut wire = profile();
        wire.document_media_types = media_types.into_iter().map(str::to_owned).collect();
        assert!(decode(vec![wire], &capabilities).is_err());
    }
}
