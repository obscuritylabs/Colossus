use super::*;
use crate::browser_tools::execution::BrowserToolExecutor;

struct NoFallback;
#[async_trait]
impl ToolExecutor for NoFallback {
    async fn execute(&self, call: ToolCall, _: ExecutionContext) -> Result<ToolResult, ToolError> {
        Err(ToolError::Unknown(call.name))
    }
}

fn tool_run(runtime: &Runtime) -> (BrowserToolExecutor, ExecutionContext) {
    let browser = runtime.browser.as_ref().unwrap();
    let context = ExecutionContext {
        run_id: Some("explicit-close".into()),
        session_id: Some("owned-conversation".into()),
        offered_tools: vec![
            "browser.open".into(),
            "browser.snapshot".into(),
            "browser.close".into(),
        ],
        ..ExecutionContext::default()
    };
    browser
        .begin_run(
            &context,
            &Actor {
                actor_type: ActorType::Application,
                id: "desktop".into(),
            },
            RunControl::default(),
        )
        .unwrap();
    (
        BrowserToolExecutor {
            gateway: Arc::clone(&runtime.gateway),
            registry: Arc::clone(&runtime.tools),
            browser: Arc::clone(browser),
            journal: runtime.journal(),
            inner: Arc::new(NoFallback),
        },
        context,
    )
}

async fn call(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    name: &str,
    arguments: Value,
) -> Result<Value, ToolError> {
    let result = executor
        .execute(
            ToolCall {
                call_id: Uuid::now_v7().to_string(),
                name: name.into(),
                arguments,
            },
            context.clone(),
        )
        .await?;
    Ok(serde_json::from_str(&result.output).unwrap())
}

async fn open_viewer(
    runtime: &Runtime,
    role: &RuntimeNativeBrowserAuthority,
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
) -> (Value, RuntimeNativeBrowserSession) {
    let opened = call(
        executor,
        context,
        "browser.open",
        json!({"mode":"embedded", "allowed_origins":["https://example.org"], "initial_url":"https://example.org/page"}),
    )
    .await
    .unwrap();
    call(
        executor,
        context,
        "browser.snapshot",
        json!({
            "session_id": opened["session"]["session_id"],
            "control_generation": opened["control_generation"],
            "tab_id": opened["session"]["tabs"][0]["tab_id"],
            "document_id": opened["session"]["tabs"][0]["document_id"],
            "max_nodes": 1
        }),
    )
    .await
    .unwrap();
    let viewer = runtime
        .watch_native_browser(role, "explicit-close", "owned-conversation")
        .await
        .unwrap();
    assert_eq!(
        viewer.session_id().as_str(),
        opened["session"]["session_id"]
    );
    (opened, viewer)
}

fn close_arguments(opened: &Value) -> Value {
    json!({
        "session_id": opened["session"]["session_id"],
        "control_generation": opened["control_generation"]
    })
}

#[tokio::test]
async fn tool_close_retires_only_its_viewer_before_same_run_reopen_and_completion() {
    let home = private_tempdir();
    let driver = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), driver.clone(), true);
    let role = role(&runtime);
    let (executor, context) = tool_run(&runtime);
    let (first, viewer) = open_viewer(&runtime, &role, &executor, &context).await;
    let unrelated = runtime
        .open_native_browser(&role, input(None))
        .await
        .unwrap();
    let receipt = call(
        &executor,
        &context,
        "browser.close",
        close_arguments(&first),
    )
    .await
    .unwrap();
    assert_eq!(
        receipt,
        json!({"session_id":viewer.session_id(), "closed":true})
    );
    assert!(
        !runtime
            .browser
            .as_ref()
            .unwrap()
            .run(&context)
            .unwrap()
            .sessions
            .contains_key(viewer.session_id())
    );
    assert!(
        viewer.is_closed().unwrap(),
        "positive tool cleanup must retire its retained viewer before forgetting the session"
    );
    assert!(!unrelated.is_closed().unwrap());
    assert_eq!(driver.closed.load(Ordering::SeqCst), 1);

    let (_, second) = open_viewer(&runtime, &role, &executor, &context).await;
    assert_ne!(viewer.session_id(), second.session_id());
    assert!(!second.is_closed().unwrap());
    runtime
        .browser
        .as_ref()
        .unwrap()
        .finish_run("explicit-close")
        .await;
    assert!(viewer.is_closed().unwrap());
    assert!(second.is_closed().unwrap());
    assert!(!unrelated.is_closed().unwrap());
    assert_eq!(driver.closed.load(Ordering::SeqCst), 2);
    runtime
        .close_native_browser(&role, unrelated.session_id())
        .await
        .unwrap();
    assert!(unrelated.is_closed().unwrap());
    assert!(driver.hosts.lock().unwrap().is_empty());
    assert!(!runtime.browser_active_work());
}

#[tokio::test]
async fn tool_close_keeps_uncertain_native_cleanup_owned_and_unconfirmed() {
    let home = private_tempdir();
    let driver = Arc::new(Fixture::default());
    let runtime = fixture_runtime(home.path(), driver.clone(), true);
    let role = role(&runtime);
    let (executor, context) = tool_run(&runtime);
    let (opened, viewer) = open_viewer(&runtime, &role, &executor, &context).await;
    driver.close_unknown.store(true, Ordering::SeqCst);
    assert!(matches!(
        call(
            &executor,
            &context,
            "browser.close",
            close_arguments(&opened)
        )
        .await,
        Err(ToolError::OutcomeUnknown(_))
    ));
    assert!(!viewer.is_closed().unwrap());
    let browser = runtime.browser.as_ref().unwrap();
    assert!(
        browser
            .run(&context)
            .unwrap()
            .sessions
            .contains_key(viewer.session_id())
    );
    assert!(
        browser
            .native
            .entries
            .lock()
            .unwrap()
            .contains_key(&viewer.entry.id)
    );
    assert!(
        browser
            .native
            .agent_requests
            .lock()
            .unwrap()
            .contains_key(viewer.session_id())
    );
    assert!(
        driver
            .hosts
            .lock()
            .unwrap()
            .contains_key(viewer.session_id())
    );
    assert_eq!(driver.closed.load(Ordering::SeqCst), 0);
    assert!(runtime.browser_active_work());

    driver.close_unknown.store(false, Ordering::SeqCst);
    runtime.drain_browser_sessions().await;
    assert!(viewer.is_closed().unwrap());
    assert!(driver.hosts.lock().unwrap().is_empty());
    assert!(!runtime.browser_active_work());
}
