use super::*;

pub(super) async fn oversized_context_reaches_public_failure(runtime: Arc<Runtime>) {
    let service = service(Arc::clone(&runtime), RunAdmissionConfig::default());
    for (bytes, expected_code, expected_resource) in [
        (
            128 * 1024,
            "context.token_limit_exceeded",
            "estimated tokens",
        ),
        (896 * 1024, "context.request_too_large", "bytes"),
    ] {
        let application_id = format!("app:context-{}", Uuid::now_v7().simple());
        let owner = caller(&application_id, "context-size-create");
        let prompt = "x".repeat(bytes);
        let created = service
            .create_run(
                &owner,
                request(&format!("context-{}", Uuid::now_v7().simple()), &prompt),
            )
            .await
            .expect("public input below admission limit")
            .run;
        let terminal = wait_terminal(&service, &owner, &created.id).await;
        assert_eq!(terminal.status, RunStatus::Failed);
        let failure = terminal.failure.expect("released context failure");
        assert_eq!(failure.code, expected_code);
        assert!(failure.message.contains(expected_resource));
        assert!(failure.message.contains("shorten the message"));
        assert!(!failure.message.contains("known outcome"));
        assert_eq!(failure.outcome, OutcomeCertainty::Known);
        assert!(!failure.recoverable);

        let mut updates = service
            .watch_run(
                &owner,
                WatchRunRequest {
                    run_id: created.id,
                    after_sequence: 0,
                },
            )
            .await
            .expect("replay public failure evidence");
        let mut saw_notice = false;
        while let Some(update) = updates.next().await {
            if let RunUpdateKind::Notice { notice } = update.unwrap().kind
                && notice.reason == expected_code
            {
                assert!(!notice.message.contains("encountered an error"));
                saw_notice = true;
            }
        }
        assert!(
            saw_notice,
            "context failure must also explain live progress"
        );
        let events = runtime
            .journal()
            .read_stream(&format!("run:{}", terminal.id))
            .expect("run events");
        let error = events
            .iter()
            .find(|event| event.event_type == "error.v1")
            .expect("numeric context failure evidence");
        let payload = runtime.journal().decrypt_payload(error).unwrap();
        assert_eq!(payload["code"], expected_code);
        assert!(payload["required"].as_u64().unwrap() > payload["limit"].as_u64().unwrap());
        assert_eq!(payload["resource"], expected_resource);
    }
}
