use super::*;

#[tokio::test]
async fn browser_observations_cannot_disable_release_with_external_policy() {
    for action in ["browser.snapshot", "browser.click", "browser.open"] {
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
