//! Retained inventories preserve project authorization and exclusive event cursors.
use super::*;

pub(super) async fn exercise(
    client: &reqwest::Client,
    origin: &str,
    headers: &HeaderMap,
    task: &str,
) {
    let path = format!("{origin}/api/projects/project-a/tasks/{task}/updates");
    let anonymous = client.get(&path).send().await.unwrap();
    assert_eq!(anonymous.status(), 403);
    let foreign = client
        .get(format!(
            "{origin}/api/projects/project-b/tasks/{task}/updates"
        ))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), 403);
    let malformed = client
        .get(format!("{path}?after=invalid"))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), 400);
    let mut after = 0;
    let mut seen = 0;
    loop {
        let response = client
            .get(format!("{path}?after={after}"))
            .headers(headers.clone())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let bytes = response.bytes().await.unwrap();
        assert!(bytes.len() <= colossus_cloud_protocol::MAX_PAYLOAD_BYTES);
        let page: Value = serde_json::from_slice(&bytes).unwrap();
        let updates = page["updates"].as_array().unwrap();
        assert!(updates.len() <= 32);
        for update in updates {
            let sequence = update["sequence"].as_u64().unwrap();
            assert!(sequence > after, "exclusive monotonic cursor");
            after = sequence;
            seen += 1;
        }
        if page["next_after"].is_null() {
            break;
        }
        assert!(!updates.is_empty());
        assert_eq!(page["next_after"].as_u64(), Some(after));
    }
    assert!(seen > 0);
    let end: Value = client
        .get(format!("{path}?after={after}"))
        .headers(headers.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(end["updates"].as_array().unwrap().is_empty());
    assert!(end["next_after"].is_null());
    let future = client
        .get(format!("{path}?after={}", u64::MAX))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(future.status(), 409);
}
