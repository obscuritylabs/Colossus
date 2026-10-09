//! Operator-driven `WebView2` smoke test, isolated from user settings and runtime.
//! These fixture decisions never execute a command or create saved consent.
use super::*;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires interactive Windows; click Allow once, Always allow, then close"]
fn native_review_buttons_and_close() {
    let profile = tempfile::tempdir().unwrap();
    let mut context = crate::app_context::create();
    context.config_mut().identifier =
        format!("com.colossus.approval-test.{}", uuid::Uuid::now_v7());
    context.config_mut().app.windows.clear();
    let result = Arc::new(Mutex::new(None));
    let outcome = result.clone();
    let directory = profile.path().to_owned();
    let app = tauri::Builder::default()
        .any_thread()
        .register_uri_scheme_protocol(
            crate::command_review_protocol::SCHEME,
            |context, request| crate::command_review_protocol::respond(&context, &request),
        )
        .manage(CommandReviewState::default())
        .invoke_handler(tauri::generate_handler![
            command_review_context,
            finish_command_review
        ])
        .setup(move |app| {
            let app = app.handle().clone();
            std::thread::spawn(move || {
                *outcome.lock().unwrap() = Some(check_choices(&app, &directory));
                app.exit(0);
            });
            Ok(())
        })
        .build(context)
        .unwrap();
    app.run_return(|_, event| {
        if let tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
        }
    });
    result
        .lock()
        .unwrap()
        .take()
        .expect("smoke test completed")
        .unwrap();
}

fn check_choices(app: &AppHandle, directory: &std::path::Path) -> Result<(), String> {
    for (step, expected) in [
        ("Click Allow once", Some(ApprovalChoice::AllowOnce)),
        ("Click Always allow", Some(ApprovalChoice::AlwaysAllow)),
        ("Close this window", None),
    ] {
        let review_id = uuid::Uuid::new_v4().to_string();
        let (send, mut receive) = oneshot::channel();
        *app.state::<CommandReviewState>().0.lock().unwrap() = Some(PendingReview {
            details: CommandReviewDto {
                review_id: review_id.clone(),
                target: "Native UI test only — no command will execute".into(),
                command_context: Some(
                    colossus_sdk::CommandApprovalContext {
                        justification: step.into(),
                        executable: "example-build-tool.exe".into(),
                        arguments: vec!["--check".into(), "COMMAND_TAIL".into()],
                        working_directory: "C:\\isolated-test-workspace".into(),
                        redacted: false,
                    }
                    .into(),
                ),
                action: "process.execute".into(),
                resource: "UI fixture".into(),
                reason: step.into(),
                can_remember: true,
            },
            response: Some(send),
        });
        // Exercise the shipped custom protocol even though this is a debug test.
        let url = format!(
            "{}://localhost/index.html?surface=command-approval",
            crate::command_review_protocol::SCHEME
        );
        let window = WebviewWindowBuilder::new(
            app,
            WINDOW,
            tauri::WebviewUrl::CustomProtocol(url.parse().unwrap()),
        )
        .use_https_scheme(false)
        .on_navigation(crate::command_review_protocol::navigation_allowed)
        .data_directory(directory.to_owned())
        .title(format!("Colossus approval test — {step}"))
        .inner_size(900.0, 700.0)
        .build()
        .map_err(|error| error.to_string())?;
        let application = app.clone();
        let current_id = review_id.clone();
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                cancel_review(&application, &current_id);
            }
        });

        let began = Instant::now();
        let decision = loop {
            match receive.try_recv() {
                Ok(choice) => break Some(choice),
                Err(oneshot::error::TryRecvError::Closed) => break None,
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
            if began.elapsed() > Duration::from_mins(3) {
                return Err(format!("operator did not complete: {step}"));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        if decision != expected {
            return Err(format!("unexpected decision for {step}: {decision:?}"));
        }
        cancel_review(app, &review_id);
        if decision.is_some() {
            check_navigation(&window)?;
            window.close().map_err(|error| error.to_string())?;
        }
        let began = Instant::now();
        while app.get_webview_window(WINDOW).is_some() {
            if began.elapsed() > Duration::from_secs(5) {
                return Err("review window did not close".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        println!("verified native review: {step}");
    }
    Ok(())
}

fn check_navigation(window: &tauri::WebviewWindow) -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(800));
    window
        .navigate("http://127.0.0.1:9/untrusted".parse().unwrap())
        .map_err(|error| error.to_string())?;
    std::thread::sleep(Duration::from_millis(200));
    let current = window.url().map_err(|error| error.to_string())?;
    if !crate::command_review_protocol::navigation_allowed(&current) {
        return Err(format!(
            "review navigated away from its fixed document: {current}"
        ));
    }
    Ok(())
}
