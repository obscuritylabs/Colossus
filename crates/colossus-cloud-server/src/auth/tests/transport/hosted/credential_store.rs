use super::*;
use colossus_cloud_chat::{ChatConnectionKind, ChatCredentialResolver, CredentialStore};
use colossus_contracts::VaultRecord;
use colossus_ports::CredentialKey;
use reqwest::StatusCode;

pub(super) async fn exercise(
    client: &reqwest::Client,
    origin: &str,
    alice: &HeaderMap,
    bob: &HeaderMap,
    state: &Arc<crate::server::State>,
) {
    let base = "/api/projects/project-a/chat/credential-store/personal";
    let caller = state.auth.caller(alice, "project-a", false).await.unwrap();
    let host = state.hosted.as_ref().unwrap();
    let config = host.project_config("project-a").await.unwrap().unwrap();
    let binding = host
        .connection_binding(&caller, &config, ChatConnectionKind::Provider, "personal")
        .unwrap()
        .unwrap();
    let vault = host.user_vault("project-a", caller.subject()).unwrap();
    let raw_key = CredentialKey::new("cloud-chat", &binding.record_id()).unwrap();
    vault
        .write(
            &raw_key,
            &VaultRecord::new(b"retained-fixture-secret".to_vec()).unwrap(),
        )
        .unwrap();
    let legacy = get(client, origin, alice, base).await;
    assert_eq!(legacy["credentials"][0]["legacy"], true);
    assert!(!legacy.to_string().contains("retained-fixture-secret"));
    let id = legacy["credentials"][0]["id"].as_str().unwrap();
    let rotated = post(client, origin, alice, &format!("{base}/{id}"), json!({
        "revision":legacy["revision"], "label":"My retained account", "kind":"api_key", "secret":"rotated-fixture-secret"
    })).await;
    assert_eq!(
        CredentialStore::resolve_bound(vault.as_ref(), &binding)
            .unwrap()
            .unwrap()
            .expose(),
        b"rotated-fixture-secret"
    );
    let response = client
        .delete(format!("{origin}{base}/{id}"))
        .headers(alice.clone())
        .json(&json!({"revision":rotated["revision"]}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert!(!ChatCredentialResolver::contains(vault.as_ref(), &binding).unwrap());
    let empty = get(client, origin, alice, base).await;
    let created = post(client, origin, alice, base, json!({
        "revision":empty["revision"], "label":"Unused key", "kind":"bearer_token", "secret":"new-fixture-secret"
    })).await;
    let inventory = get(client, origin, alice, base).await;
    assert_eq!(inventory["credentials"][0]["used_by"], json!([]));
    assert!(!inventory.to_string().contains("new-fixture-secret"));
    assert!(!ChatCredentialResolver::contains(vault.as_ref(), &binding).unwrap());
    let attach = format!("{base}/attachments/provider/personal");
    let stale = client.post(format!("{origin}{attach}")).headers(alice.clone()).json(&json!({
        "revision":created["revision"],"credential_id":created["id"],"credential_binding":"wrong-destination"
    })).send().await.unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let attached = post(client, origin, alice, &attach, json!({
        "revision":created["revision"],"credential_id":created["id"],"credential_binding":binding.record_id()
    })).await;
    assert_eq!(
        CredentialStore::resolve_bound(vault.as_ref(), &binding)
            .unwrap()
            .unwrap()
            .expose(),
        b"new-fixture-secret"
    );
    let bob_inventory = get(client, origin, bob, base).await;
    assert_eq!(bob_inventory["credentials"], json!([]));
    let bob_binding = bob_inventory["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "personal")
        .unwrap()["credential_binding"]
        .clone();
    let foreign = client.post(format!("{origin}{attach}")).headers(bob.clone()).json(&json!({
        "revision":bob_inventory["revision"],"credential_id":created["id"],"credential_binding":bob_binding
    })).send().await.unwrap();
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    let denied = client
        .post(format!(
            "{origin}/api/projects/project-a/chat/credential-store/project"
        ))
        .headers(bob.clone())
        .json(&json!({"revision":0,"label":"Forbidden","kind":"api_key","secret":"fixture"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let stale_revision = client.post(format!("{origin}{attach}")).headers(alice.clone()).json(&json!({
        "revision":created["revision"],"credential_id":null,"credential_binding":binding.record_id()
    })).send().await.unwrap();
    assert_eq!(stale_revision.status(), StatusCode::CONFLICT);
    let response = client
        .delete(format!(
            "{origin}{base}/{}",
            created["id"].as_str().unwrap()
        ))
        .headers(alice.clone())
        .json(&json!({"revision":attached["revision"]}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert!(!ChatCredentialResolver::contains(vault.as_ref(), &binding).unwrap());
    let accounts = get(
        client,
        origin,
        alice,
        "/api/projects/project-a/chat/accounts",
    )
    .await;
    assert_eq!(accounts["accounts"][0]["status"], "setup_required");
    assert_eq!(accounts["accounts"][0]["can_connect"], false);
}

pub(super) async fn cross_replica(
    client: &reqwest::Client,
    origin_a: &str,
    origin_b: &str,
    alice: &HeaderMap,
    bob: &HeaderMap,
) {
    let base = "/api/projects/project-a/chat/credential-store/personal";
    let initial = get(client, origin_a, alice, base).await;
    let created = post(
        client,
        origin_a,
        alice,
        base,
        json!({"revision":initial["revision"],
        "label":"Across replicas","kind":"api_key","secret":"synthetic-shared-vault-key"}),
    )
    .await;
    let read_b = get(client, origin_b, alice, base).await;
    assert_eq!(read_b["credentials"][0]["id"], created["id"]);
    assert_eq!(read_b["credentials"][0]["used_by"], json!([]));
    assert!(!read_b.to_string().contains("synthetic-shared-vault-key"));
    let binding = read_b["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "personal")
        .unwrap()["credential_binding"]
        .clone();
    let attached = post(
        client,
        origin_b,
        alice,
        &format!("{base}/attachments/provider/personal"),
        json!({
        "revision":read_b["revision"],"credential_id":created["id"],"credential_binding":binding}),
    )
    .await;
    let read_a = get(client, origin_a, alice, base).await;
    assert_eq!(read_a["credentials"][0]["used_by"][0]["id"], "personal");
    assert_eq!(
        get(client, origin_b, bob, base).await["credentials"],
        json!([])
    );
    let removed = client
        .delete(format!(
            "{origin_a}{base}/{}",
            created["id"].as_str().unwrap()
        ))
        .headers(alice.clone())
        .json(&json!({"revision":attached["revision"]}))
        .send()
        .await
        .unwrap();
    assert!(removed.status().is_success());
    assert_eq!(
        get(client, origin_b, alice, base).await["credentials"],
        json!([])
    );
    let base = "/api/projects/project-a/chat/credential-store/project";
    let initial = get(client, origin_a, alice, base).await;
    assert_eq!(initial["connections"], json!([]));
    let created = post(client, origin_b, alice, base, json!({"revision":initial["revision"],
        "label":"Project key before connection","kind":"bearer_token","secret":"synthetic-project-vault-key"})).await;
    assert_eq!(
        get(client, origin_a, alice, base).await["credentials"][0]["id"],
        created["id"]
    );
}
