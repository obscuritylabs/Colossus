//! Hosted inventory is personal, cold-readable and independent of Fleet enrollment.
use super::*;
const BASE: &str = "/api/projects/project-a/hosted";

pub(super) async fn cold(
    client: &reqwest::Client,
    origin: &str,
    alice: &HeaderMap,
    state: &Arc<crate::server::State>,
) {
    let inventory = get(client, origin, alice, BASE).await;
    assert_eq!(inventory["enabled"], true);
    assert_eq!(inventory["target"]["execution"]["state"], "on_demand");
    assert_eq!(
        inventory["activity"],
        json!({"queued":0,"running":0,"waiting":0,"limited":false})
    );
    assert!(
        get(client, origin, alice, &format!("{BASE}/tasks")).await["tasks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(state.hosted.as_ref().unwrap().live.lock().await.is_empty());
    for query in [
        "owner_user_id=other",
        "node_id=other",
        "limit=0",
        "limit=101",
    ] {
        assert_eq!(
            client
                .get(format!("{origin}{BASE}/tasks?{query}"))
                .headers(alice.clone())
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
    let mut no_csrf = alice.clone();
    no_csrf.remove("x-colossus-csrf");
    for action in ["load", "release"] {
        assert_eq!(
            client
                .post(format!("{origin}{BASE}/runtime/{action}"))
                .headers(no_csrf.clone())
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
}

pub(super) async fn lifecycle(
    client: &reqwest::Client,
    origin: &str,
    alice: &HeaderMap,
    bob: &HeaderMap,
) {
    let before = get(client, origin, alice, BASE).await;
    post(
        client,
        origin,
        alice,
        &format!("{BASE}/runtime/release"),
        json!({}),
    )
    .await;
    let cold = get(client, origin, alice, BASE).await;
    assert_eq!(
        cold["target"]["node"]["node_id"],
        before["target"]["node"]["node_id"]
    );
    assert_eq!(cold["target"]["execution"]["state"], "on_demand");
    assert_eq!(
        get(client, origin, bob, BASE).await["target"]["execution"]["state"],
        "warm"
    );
    post(
        client,
        origin,
        alice,
        &format!("{BASE}/runtime/load"),
        json!({}),
    )
    .await;
    assert_eq!(
        get(client, origin, alice, BASE).await["target"]["execution"]["state"],
        "warm"
    );
}

pub(super) async fn history(
    client: &reqwest::Client,
    origin: &str,
    alice: &HeaderMap,
    bob: &HeaderMap,
) {
    let first = get(client, origin, alice, &format!("{BASE}/tasks?limit=1")).await;
    assert_eq!(first["tasks"].as_array().unwrap().len(), 1);
    let cursor = first["next_cursor"].as_str().unwrap();
    let next_path = format!("{BASE}/tasks?limit=1&after={cursor}");
    let second = get(client, origin, alice, &next_path).await;
    assert_eq!(second["tasks"].as_array().unwrap().len(), 1);
    assert_ne!(first["tasks"][0]["task_id"], second["tasks"][0]["task_id"]);
    assert!(
        get(client, origin, bob, &format!("{BASE}/tasks")).await["tasks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        client
            .get(format!("{origin}{next_path}"))
            .headers(bob.clone())
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let inventory = get(client, origin, alice, BASE).await;
    assert_eq!(inventory["activity"]["queued"], 0);
    assert_eq!(inventory["activity"]["waiting"], 0);
}

pub(super) async fn busy(client: &reqwest::Client, origin: &str, alice: &HeaderMap, task: &str) {
    let inventory = get(client, origin, alice, BASE).await;
    assert_eq!(inventory["activity"]["queued"], 1);
    let active = get(client, origin, alice, &format!("{BASE}/tasks?active=true")).await;
    assert_eq!(active["tasks"][0]["task_id"], task);
    assert_eq!(
        client
            .post(format!("{origin}{BASE}/runtime/release"))
            .headers(alice.clone())
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
}

pub(super) async fn cross_replica(
    client: &reqwest::Client,
    origin_a: &str,
    origin_b: &str,
    alice: &HeaderMap,
) {
    assert_eq!(
        get(client, origin_b, alice, BASE).await["target"]["execution"]["state"],
        "warm"
    );
    post(
        client,
        origin_b,
        alice,
        &format!("{BASE}/runtime/release"),
        json!({}),
    )
    .await;
    for origin in [origin_a, origin_b] {
        assert_eq!(
            get(client, origin, alice, BASE).await["target"]["execution"]["state"],
            "on_demand"
        );
    }
    post(
        client,
        origin_a,
        alice,
        &format!("{BASE}/runtime/load"),
        json!({}),
    )
    .await;
    assert_eq!(
        get(client, origin_b, alice, BASE).await["target"]["execution"]["state"],
        "warm"
    );
}
