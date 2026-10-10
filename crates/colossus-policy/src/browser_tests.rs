use super::*;
use crate::DenyApproval;
use base64::engine::general_purpose::STANDARD as BASE64;
use colossus_contracts::{EffectPhase, EffectRequest};

#[test]
fn complete_upload_policy_projection_preserves_actual_four_mib_and_other_caps() {
    use base64::Engine as _;
    let bytes = vec![7_u8; 4 * 1024 * 1024];
    let request = effect_request(
        system_actor("upload-bound-test"),
        "browser.upload",
        "browser:session",
        serde_json::json!({"content_base64":BASE64.encode(&bytes),"size":bytes.len(),"media_type":"application/octet-stream"}),
    );
    let kernel = SafetyKernel::new([]);
    let projected = kernel
        .policy_projection(&request)
        .expect("closed upload projection fits default byte cap");
    assert_eq!(
        BASE64
            .decode(projected.content["content_base64"].as_str().unwrap())
            .unwrap(),
        bytes
    );
    let mut ordinary = request.clone();
    ordinary.action = "provider.echo".into();
    assert!(
        matches!(kernel.policy_projection(&ordinary),Err(GatewayError::Policy(PolicyError::InputTooLarge{limit})) if limit==1024*1024)
    );
    let strict = SafetyKernel::new([]).with_policy_input_limit(1024);
    assert!(matches!(
        strict.policy_projection(&request),
        Err(GatewayError::Policy(PolicyError::InputTooLarge {
            limit: 1024
        }))
    ));
    let mut post = request;
    post.phase = EffectPhase::PostEffect;
    assert!(matches!(
        strict.policy_projection(&post),
        Err(GatewayError::Policy(PolicyError::InputTooLarge {
            limit: 1024
        }))
    ));
}

#[tokio::test]
async fn custom_upload_input_cap_denies_before_native_adapter() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Native(AtomicUsize);
    #[async_trait]
    impl EffectExecutor for Native {
        async fn execute(
            &self,
            _: &EffectRequest,
            _: ExecutionPermit,
        ) -> Result<QuarantinedEffectResult, ExecutionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(ExecutionError::Failed("unexpected native dispatch".into()))
        }
    }
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        journal,
        Arc::new(
            BuiltInPolicy::offline_default().with_action("browser.upload", DecisionOutcome::Allow),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new([]).with_policy_input_limit(512),
        [9; 32],
    );
    let native = Native(AtomicUsize::new(0));
    let result = gateway
        .execute(
            effect_request(
                system_actor("upload-bound-test"),
                "browser.upload",
                "browser:session",
                serde_json::json!({"content_base64":"a".repeat(1024)}),
            ),
            &native,
        )
        .await;
    assert!(matches!(
        result,
        Err(GatewayError::Policy(PolicyError::InputTooLarge {
            limit: 512
        }))
    ));
    assert_eq!(native.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn browser_observations_cannot_disable_release_with_external_policy() {
    for action in [
        "browser.snapshot",
        "browser.screenshot",
        "browser.upload",
        "browser.download",
        "browser.click",
        "browser.open",
    ] {
        let request = effect_request(
            system_actor("browser-release-test"),
            action,
            "browser:session",
            serde_json::json!({}),
        );
        let mut decision = BuiltInPolicy::offline_default()
            .with_action(action, DecisionOutcome::Allow)
            .decide(&request)
            .await
            .expect("decision");
        let kernel = SafetyKernel::new([action.into()]);
        decision.obligations.require_post_effect = false;
        assert!(matches!(
            kernel.validate_decision(&request, &decision),
            Err(GatewayError::Safety(_))
        ));
        decision.obligations.require_post_effect = true;
        kernel
            .validate_decision(&request, &decision)
            .expect("release obligation accepted");
    }
}

#[tokio::test]
async fn unavailable_browser_does_not_gain_implicit_policy_authority() {
    let decision = BuiltInPolicy::offline_default()
        .decide(&effect_request(
            system_actor("browser-default-test"),
            "browser.open",
            "browser:session",
            serde_json::json!({}),
        ))
        .await
        .expect("decision");
    assert_eq!(decision.outcome, DecisionOutcome::Deny);
}
