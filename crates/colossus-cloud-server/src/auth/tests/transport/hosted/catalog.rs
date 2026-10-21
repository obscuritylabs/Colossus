//! Browser catalog admission is scoped, revision-bound and never accepts secrets or endpoints.
use super::*;

pub(super) async fn acceptance(
    client: &reqwest::Client,
    origin: &str,
    alice: &HeaderMap,
    bob: &HeaderMap,
) {
    let presets = get(
        client,
        origin,
        alice,
        "/api/projects/project-a/chat/settings/provider-presets",
    )
    .await;
    assert_eq!(
        presets["presets"],
        json!(colossus_contracts::provider_presets())
    );
    assert_eq!(presets["availability"].as_object().unwrap().len(), 12);
    for id in ["codex", "openai", "anthropic", "ollama", "lmstudio"] {
        assert_eq!(presets["availability"][id]["status"], "deployment_required");
        assert!(
            presets["availability"][id]["reason"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
    }
    assert_eq!(
        presets["availability"]["custom-chat"]["status"],
        "configure_endpoint"
    );
    assert!(!presets.to_string().contains("provider.example.test"));
    let path = "/api/projects/project-a/chat/settings/providers/echo/models";
    let models = post(client, origin, alice, path, json!({"revision":1})).await;
    assert_eq!(models["models"][0]["id"], "echo");
    let mut no_csrf = alice.clone();
    no_csrf.remove("x-colossus-csrf");
    for (headers, body, status) in [
        (bob, json!({"revision":1}), 403),
        (&no_csrf, json!({"revision":1}), 403),
        (alice, json!({"revision":0}), 409),
        (
            alice,
            json!({"revision":1,"secret":"must-not-be-accepted"}),
            422,
        ),
        (
            alice,
            json!({"revision":1,"endpoint":"https://unapproved.example"}),
            422,
        ),
    ] {
        assert_eq!(
            client
                .post(format!("{origin}{path}"))
                .headers(headers.clone())
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            status
        );
    }
    assert_eq!(
        client
            .post(format!(
                "{origin}/api/projects/project-a/chat/settings/providers/missing/models"
            ))
            .headers(alice.clone())
            .json(&json!({"revision":1}))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!(
                "{origin}/api/projects/project-a/chat/settings/provider-presets"
            ))
            .headers(bob.clone())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
}
