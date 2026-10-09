//! Exercise management through HTTP, shared storage, mTLS and the public workflow SDK.
use super::*;
pub(super) async fn exercise(
    client: &reqwest::Client,
    origin: &str,
    headers: &axum::http::HeaderMap,
    node: &str,
) {
    let path = format!("/api/projects/project-a/nodes/{node}/resources");
    let context = post(
        client,
        origin,
        headers,
        &path,
        json!({"connection_id":null,"operation":{"operation":"context"}}),
    )
    .await;
    assert_eq!(context["kind"], "result");
    let connection = context["connection_id"].as_str().unwrap();
    let send = |operation: serde_json::Value| {
        post(
            client,
            origin,
            headers,
            &path,
            json!({"connection_id":connection,"operation":operation}),
        )
    };
    let yaml = "apiVersion: colossus.dev/v1alpha1\nkind: Workflow\nmetadata:\n  name: public-health\n  version: 1.0.0\n  description: Public workflow fixture\ninputs: {type: object, additionalProperties: false}\noutputs: {type: object}\ncapabilities: []\nmaxConcurrency: 1\nstepBudget: 2\nsteps:\n  - id: result\n    type: emit\n    value: {ok: true}\n";
    let validated = send(json!({"operation":"validate_workflow","yaml":yaml})).await;
    assert_eq!(validated["kind"], "result");
    let hash = validated["value"]["workflow_hash"].as_str().unwrap();
    let registered=send(json!({"operation":"register_workflow","yaml":yaml,"expected_hash":hash,"idempotency_key":"register-resource"})).await;
    assert_eq!(registered["kind"], "result");
    let workflow = registered["value"]["workflow_id"].as_str().unwrap();
    assert_eq!(
        send(json!({"operation":"get_workflow","id":workflow})).await["value"]["workflow_hash"],
        hash
    );
    assert_eq!(
        send(json!({"operation":"list_workflows","after":null})).await["value"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let schedule=send(json!({"operation":"create_schedule","request":{"schedule_id":"resource-health","workflow_id":workflow,"expected_hash":hash,"inputs":{},"cadence_seconds":60,"calendar":null,"task":null,"starts_at":"2030-10-09T12:00:00Z","misfire_policy":"skip","enabled":false,"idempotency_key":"schedule-resource"}})).await;
    assert_eq!(schedule["kind"], "result");
    let etag = schedule["value"]["etag"].as_str().unwrap();
    assert_eq!(
        send(json!({"operation":"list_schedules","after":null})).await["value"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        send(json!({"operation":"get_schedule","id":"resource-health"})).await["value"]["etag"],
        etag
    );
    let enabled=send(json!({"operation":"set_schedule_enabled","request":{"schedule_id":"resource-health","enabled":true,"etag":etag}})).await;
    assert_eq!(enabled["value"]["record"]["enabled"], true);
    let stale=send(json!({"operation":"set_schedule_enabled","request":{"schedule_id":"resource-health","enabled":false,"etag":etag}})).await;
    assert_eq!(stale["kind"], "failed");
    assert_eq!(stale["error"]["code"], "conflict");
    let run=send(json!({"operation":"start_workflow_run","request":{"workflow_id":workflow,"expected_hash":hash,"inputs":{},"idempotency_key":"run-resource"}})).await;
    assert_eq!(run["kind"], "result");
    let id = run["value"]["run_id"].as_str().unwrap();
    assert_eq!(
        send(json!({"operation":"get_workflow_run","id":id})).await["value"]["run_id"],
        id
    );
    assert_eq!(send(json!({"operation":"list_workflow_runs","workflow_id":workflow,"after":null})).await["value"]["items"].as_array().unwrap().len(),1);
    assert_eq!(send(json!({"operation":"delete_schedule","request":{"schedule_id":"resource-health","etag":enabled["value"]["etag"]}})).await["value"]["schedule_id"],"resource-health");
    assert!(
        send(json!({"operation":"list_schedules","after":null})).await["value"]["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let old=client.post(format!("{origin}{path}")).headers(headers.clone()).json(&json!({"connection_id":"stale-connection","operation":{"operation":"list_schedules","after":null}})).send().await.unwrap();
    assert_eq!(old.status(), 409);
    let arbitrary=client.post(format!("{origin}{path}")).headers(headers.clone()).json(&json!({"connection_id":connection,"operation":{"operation":"invoke_sdk","method":"worker.inspect_session_map"}})).send().await.unwrap();
    assert_eq!(arbitrary.status(), 422);
}
