use super::*;

fn resources() -> ExecutionResources {
    ExecutionResources {
        capabilities: vec![
            "agent_runs.model_selection".into(),
            "agent_runs.goal".into(),
        ],
        model_profiles: vec![colossus_sdk::ConfiguredModelProfile {
            profile: "primary-model".into(),
            model: "provider-model".into(),
            provider: "responses".into(),
            provider_profile: "work".into(),
            reasoning_effort: Some("high".into()),
            default_roles: vec!["primary".into()],
            document_media_types: Vec::new(),
        }],
        ..ExecutionResources::default()
    }
}

#[tokio::test]
async fn model_catalog_forwards_only_captured_public_metadata() {
    let ResourceReply::Result { value } = resources()
        .execute(ResourceOperation::ModelCatalog {})
        .await
    else {
        panic!("catalog should be available");
    };
    assert_eq!(value["capabilities"][0], "agent_runs.model_selection");
    assert_eq!(
        value["model_profiles"][0],
        serde_json::json!({
            "profile": "primary-model",
            "model": "provider-model",
            "provider": "responses",
            "provider_profile": "work",
            "reasoning_effort": "high",
            "default_roles": ["primary"],
        })
    );
    let ResourceReply::Result { value } = resources().execute(ResourceOperation::Context).await
    else {
        panic!("context should be available");
    };
    assert!(
        value.get("model_profiles").is_none(),
        "read-only context must omit configured models"
    );
    assert!(
        value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == colossus_cloud_protocol::MODEL_CATALOG_CAPABILITY)
    );
}

#[tokio::test]
async fn model_catalog_never_invents_choices_for_older_or_unsupported_targets() {
    let mut unsupported = resources();
    unsupported.capabilities.clear();
    for resource in [unsupported, ExecutionResources::default()] {
        let ResourceReply::Result { value } =
            resource.execute(ResourceOperation::ModelCatalog {}).await
        else {
            panic!("empty catalog should remain readable");
        };
        assert_eq!(value["model_profiles"], serde_json::json!([]));
    }
}
