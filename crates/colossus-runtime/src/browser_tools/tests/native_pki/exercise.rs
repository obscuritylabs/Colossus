use super::*;

fn context(runtime: &Runtime, run: &str, scope: BrowserScope) -> ExecutionContext {
    let (session_id, workflow_id) = match scope {
        BrowserScope::Conversation { id } => (Some(id), None),
        BrowserScope::Workflow { id } => (None, Some(id)),
    };
    let context = ExecutionContext {
        run_id: Some(run.into()),
        session_id,
        workflow_id,
        offered_tools: runtime
            .tools
            .list_specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect(),
        ..ExecutionContext::default()
    };
    runtime
        .browser
        .as_ref()
        .unwrap()
        .begin_run(
            &context,
            &Actor {
                actor_type: ActorType::User,
                id: "authenticated-app".into(),
            },
            RunControl::default(),
        )
        .unwrap();
    context
}
async fn finish(runtime: &Runtime, pool: &BrowserHostPool, run: &str) {
    runtime.browser.as_ref().unwrap().finish_run(run).await;
    assert!(!runtime.browser_active_work());
    assert_eq!(pool.owned_session_count().unwrap(), 0);
}
pub(super) async fn run(
    runtime: &Runtime,
    pool: &BrowserHostPool,
    metadata: &Value,
    observed: &Arc<StdMutex<Vec<BrowserSessionBinding>>>,
    supervisor: &OciBrowserSupervisor,
    acknowledged: &AtomicUsize,
) {
    let origins = metadata["urls"]
        .as_object()
        .unwrap()
        .values()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    let tools = executor(runtime);
    let denied = context(
        runtime,
        "native-pki-mixed",
        BrowserScope::Conversation { id: "mixed".into() },
    );
    let mut mixed = origins.clone();
    mixed.push("https://unapproved.invalid");
    assert!(
        call(
            &tools,
            &denied,
            "browser.open",
            json!({"mode":"headless", "allowed_origins":mixed,
        "initial_url":format!("{}/public", metadata["urls"]["public"].as_str().unwrap())})
        )
        .await
        .is_err()
    );
    assert_eq!(
        observed.lock().unwrap().len(),
        1,
        "ordinary policy admitted the native consent check"
    );
    assert_eq!(
        pool.owned_session_count().unwrap(),
        0,
        "mixed CA trust consent allocated no native host"
    );
    finish(runtime, pool, "native-pki-mixed").await;
    for (name, endpoint, path, expected, workflow) in [
        (
            "trusted",
            "public",
            "/public",
            Some("Colossus private CA verified"),
            false,
        ),
        (
            "exact",
            "mtls",
            "/client-check",
            Some("Colossus mTLS identity verified"),
            false,
        ),
        (
            "future",
            "mtls",
            "/client-check",
            Some("Colossus mTLS identity verified"),
            false,
        ),
        (
            "alternate",
            "mtls",
            "/client-check",
            Some("Colossus identity rejected"),
            true,
        ),
        ("untrusted", "untrusted", "/public", None, false),
        ("redirect", "mtls", "/redirect-denied", None, false),
    ] {
        let run = format!("native-pki-{name}");
        let scope = if workflow {
            BrowserScope::Workflow { id: name.into() }
        } else {
            BrowserScope::Conversation { id: name.into() }
        };
        let context = context(runtime, &run, scope);
        let opened = call(
            &tools,
            &context,
            "browser.open",
            json!({"mode":"headless", "allowed_origins":origins,
            "initial_url":format!("{}{path}", metadata["urls"][endpoint].as_str().unwrap())}),
        )
        .await;
        if let Some(expected) = expected {
            let opened = opened.unwrap();
            native::loaded_fixture_snapshot(&tools, &context, &opened, expected).await;
        } else {
            assert!(
                opened.is_err(),
                "native TLS validation unexpectedly succeeded"
            );
        }
        finish(runtime, pool, &run).await;
        // Closed ledger entries are retired on the next admission. Bind each
        // positive shutdown receipt before another allocation replaces history.
        let stages = supervisor.acceptance_launch_stages().await;
        assert_eq!(stages.len(), 1);
        assert!(stages[0].1);
        assert_eq!(stages[0].2, OciBrowserShutdownReceipt::GracefulCefShutdown);
        acknowledged.fetch_add(1, Ordering::SeqCst);
    }
}
pub(super) fn server_receipts(report: &Value, metadata: &Value) {
    let events = report["events"].as_array().unwrap();
    let good = &metadata["fingerprints_sha256"]["client"];
    let alternate = &metadata["fingerprints_sha256"]["alternate_client"];
    assert!(events.iter().any(|event| event["endpoint"] == "public"
        && event["status"] == 200
        && event["client_fingerprint_sha256"].is_null()));
    assert!(
        events
            .iter()
            .filter(|event| event["endpoint"] == "mtls"
                && event["path"] == "/client-check"
                && event["status"] == 200
                && event["client_fingerprint_sha256"] == *good)
            .count()
            >= 2
    );
    assert!(events.iter().any(|event| event["endpoint"] == "mtls"
        && event["status"] == 403
        && event["client_fingerprint_sha256"] == *alternate));
    assert!(events.iter().any(|event| event["endpoint"] == "untrusted"
        && event["path"] == "tls_denied"
        && event["status"] == 0));
    assert!(events.iter().any(|event| event["endpoint"] == "mtls"
        && event["path"] == "/redirect-denied"
        && event["status"] == 302
        && event["client_fingerprint_sha256"] == *good));
    let redirect = events
        .iter()
        .filter(|event| event["endpoint"] == "redirect_mtls")
        .collect::<Vec<_>>();
    assert!(!redirect.is_empty());
    assert!(redirect.iter().all(|event| event["path"] == "tls_denied"
        && event["status"] == 0
        && event["client_fingerprint_sha256"].is_null()));
}
