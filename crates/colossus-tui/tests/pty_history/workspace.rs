use super::*;
use colossus_tui::{InteractivePrompt, InteractivePromptKind};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct WorkspaceHost {
    bootstraps: AtomicUsize,
    active: AtomicUsize,
    cancelled: AtomicUsize,
}

#[async_trait]
impl InteractiveHost for WorkspaceHost {
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        let id = request.session_id.clone().unwrap_or_else(|| {
            if self.bootstraps.fetch_add(1, Ordering::SeqCst) == 0 {
                "alpha".into()
            } else {
                "beta".into()
            }
        });
        let mut snapshot = FixtureHost.bootstrap(request).await?;
        snapshot.session_id = id.clone();
        snapshot.fresh_session = false;
        snapshot.transcript.messages.truncate(1);
        for message in &mut snapshot.transcript.messages {
            message.session_id = id.clone();
            message.message.content = format!("{id} initial conversation").into();
        }
        snapshot.completions = vec!["/sessions".into(), "/session new".into()];
        Ok(snapshot)
    }

    async fn sessions(&self) -> Result<Vec<SessionSummary>, String> {
        Ok([("alpha", "Alpha session"), ("beta", "Beta session")]
            .into_iter()
            .map(|(id, title)| fixture_session_browser_entry(id, title, 1, "").summary)
            .collect())
    }

    async fn acknowledge_sandbox_boundary(
        &self,
        _: &str,
        _: SandboxBoundaryMode,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<bool, String> {
        Ok(true)
    }

    async fn execute_command(
        &self,
        command: RuntimeCommand,
        session_id: &str,
        sticky_skills: &[String],
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostCommandResult, String> {
        FixtureHost
            .execute_command(command, session_id, sticky_skills, events, control)
            .await
    }

    async fn run_turn(
        &self,
        request: InteractiveRunRequest,
        events: mpsc::Sender<HostEvent>,
        control: RunControl,
    ) -> Result<HostRunResult, String> {
        let bit = if request.session_id == "alpha" { 1 } else { 2 };
        self.active.fetch_or(bit, Ordering::SeqCst);
        workspace_delta(
            &events,
            &request.session_id,
            &format!("{} running", request.session_id),
        )
        .await?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.active.load(Ordering::SeqCst) != 3 {
            if control.is_cancelled() || Instant::now() >= deadline {
                return Err("sessions did not overlap before cancellation or timeout".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        workspace_delta(&events, &request.session_id, "Both session futures overlap").await?;
        if bit == 1 {
            while !control.is_cancelled() {
                if Instant::now() >= deadline {
                    return Err("Alpha was not stopped".into());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            self.cancelled.fetch_or(bit, Ordering::SeqCst);
            workspace_delta(&events, "alpha", "Alpha cancellation observed").await?;
            return Err("Alpha cancellation observed".into());
        }

        let (response, mut answer) = oneshot::channel();
        events
            .send(HostEvent::Prompt(InteractivePrompt {
                id: "beta-approval".into(),
                kind: InteractivePromptKind::Approval,
                title: "Beta approval origin".into(),
                document: PresentationDocument::from_block(PresentationBlock::Text(
                    "Only Beta may receive this response".into(),
                )),
                choices: vec!["Allow once".into(), "Deny".into()],
                initial_choice: None,
                allow_free_form: false,
                response,
            }))
            .await
            .map_err(|_| "prompt receiver closed")?;
        let response = loop {
            tokio::select! {
                received = &mut answer => break received.map_err(|_| "Beta prompt dropped")?,
                () = tokio::time::sleep(Duration::from_millis(20)) => {
                    if control.is_cancelled() || Instant::now() >= deadline {
                        return Err("Beta was cancelled while Alpha was selected".into());
                    }
                }
            }
        };
        if response != PromptResponse::Answer("Allow once".into()) {
            return Err("Beta approval received the wrong response".into());
        }
        // Stop is cooperative. Allow Alpha's next cancellation boundary to run
        // even when the UI has already switched back and answered Beta.
        while self.cancelled.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            if control.is_cancelled() {
                return Err("Beta was cancelled while Alpha was selected".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if self.cancelled.load(Ordering::SeqCst) != 1 || control.is_cancelled() {
            return Err("stopping Alpha did not remain isolated".into());
        }
        let output = "Beta approval accepted while Alpha stopped";
        workspace_delta(&events, "beta", output).await?;
        Ok(HostRunResult {
            outcome: AgentRunOutcome::Completed {
                result: AgentRunResult {
                    run_id: "run-beta".into(),
                    session_id: Some("beta".into()),
                    role: "primary".into(),
                    profile: "fixture".into(),
                    model_profile: "fixture".into(),
                    provider_profile: "local".into(),
                    model: "fixture".into(),
                    plan: None,
                    output: output.into(),
                    event_count: 3,
                    elapsed_seconds: 0.1,
                },
            },
            footer: FooterState {
                role: "primary".into(),
                route: "fixture@local".into(),
                status: "ready".into(),
                approval_mode: "ask".into(),
                approval_mode_revision: 0,
                ..Default::default()
            },
            plan_selection: PlanSelectionUpdate::Unchanged,
        })
    }

    async fn run_plan_execution(
        &self,
        _: InteractivePlanExecutionRequest,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostPlanExecutionResult, String> {
        Err("workspace fixture does not execute plans".into())
    }

    async fn append_history(
        &self,
        _: &str,
        _: String,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn save_preferences(
        &self,
        _: &str,
        preferences: TerminalPreferences,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<TerminalPreferences, String> {
        Ok(preferences)
    }

    async fn older_messages(&self, _: &str, _: u64) -> Result<SessionMessagePage, String> {
        Ok(SessionMessagePage {
            messages: Vec::new(),
            before_sequence: None,
            has_more: false,
        })
    }
}

async fn workspace_delta(
    events: &mpsc::Sender<HostEvent>,
    session_id: &str,
    text: &str,
) -> Result<(), String> {
    events
        .send(HostEvent::Run(RunEventEnvelope {
            schema_version: 1,
            run_id: format!("run-{session_id}"),
            session_id: session_id.into(),
            event: RunEvent::Provider {
                event: ProviderEvent::ModelDelta {
                    text: format!("{text}\n"),
                },
            },
        }))
        .await
        .map_err(|_| "workspace receiver closed".into())
}

#[test]
fn fixture_process() {
    if std::env::var_os("COLOSSUS_TUI_WORKSPACE_FIXTURE").is_none() {
        return;
    }
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(run_tui(
            Arc::new(WorkspaceHost::default()),
            TuiOptions::default(),
        ))
        .expect("workspace TUI");
}

#[test]
fn concurrent_sessions_preserve_drafts_and_isolate_approvals_and_stop() {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("PTY");
    let mut command = fixture_command();
    command.args(["--exact", "workspace::fixture_process", "--nocapture"]);
    command.env("COLOSSUS_TUI_WORKSPACE_FIXTURE", "1");
    let mut child = pair
        .slave
        .spawn_command(command)
        .expect("workspace fixture");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let output = Arc::new(Mutex::new(Vec::new()));
    let reader_output = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => reader_output
                    .lock()
                    .expect("output")
                    .extend_from_slice(&buffer[..read]),
            }
        }
    });
    let mut writer = pair.master.take_writer().expect("writer");
    if cfg!(windows) {
        wait_for_raw(&output, b"\x1b[6n");
        write_input(&mut writer, b"\x1b[1;1R");
    }
    wait_for_screen(&output, 30, 120, "alpha initial conversation");
    write_input(&mut writer, b"alpha work\r");
    wait_for_screen(&output, 30, 120, "alpha running");
    write_input(&mut writer, b"alpha unsent");
    wait_for_screen(&output, 30, 120, "alpha unsent");

    // Opening a second session must not wait for Alpha's active future.
    center_key(&mut writer);
    wait_for_screen(&output, 30, 120, "Sessions · 1 open");
    write_input(&mut writer, b"n");
    wait_for_screen(&output, 30, 120, "beta initial conversation");
    write_input(&mut writer, b"beta work\r");
    wait_for_screen(&output, 30, 120, "Beta approval origin");

    // The command center remains accessible over an approval, preserving its
    // one-use response while the selected conversation changes.
    center_key(&mut writer);
    wait_for_screen(&output, 30, 120, "Sessions · 2 open");
    write_input(&mut writer, b"\x1b[H\r");
    wait_for_screen(&output, 30, 120, "alpha unsent");
    wait_for_screen(&output, 30, 120, "Both session futures overlap");
    let alpha = screen_contents(&output, 30, 120);
    assert!(!alpha.contains("Only Beta may receive"), "{alpha}");

    // Stop only the selected Alpha run without editing either session's draft
    // or resolving the inactive Beta approval.
    center_key(&mut writer);
    wait_for_screen(&output, 30, 120, "Sessions · 2 open");
    write_input(&mut writer, b"s");
    write_input(&mut writer, b"\x1b[F\r");
    wait_for_screen(&output, 30, 120, "Beta approval origin");
    write_input(&mut writer, b"a\r");
    wait_for_screen(
        &output,
        30,
        120,
        "Beta approval accepted while Alpha stopped",
    );

    center_key(&mut writer);
    wait_for_screen(&output, 30, 120, "Sessions · 2 open");
    write_input(&mut writer, b"\x1b[H\r");
    wait_for_screen(&output, 30, 120, "Alpha cancellation observed");
    wait_for_screen(&output, 30, 120, "alpha unsent");
    assert!(
        !screen_contents(&output, 30, 120).contains("Beta approval accepted"),
        "Beta output crossed into Alpha's transcript"
    );
    write_input(&mut writer, &[3, 3]);
    assert!(child.wait().expect("workspace fixture exit").success());
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("reader thread");
}

fn write_input(writer: &mut dyn std::io::Write, bytes: &[u8]) {
    writer.write_all(bytes).expect("PTY input");
    writer.flush().expect("flush PTY input");
}

fn center_key(writer: &mut dyn std::io::Write) {
    // ConPTY's Win32 input mode carries function keys as native input records.
    // Unix crossterm receives the corresponding xterm F2 sequence.
    let bytes: &[u8] = if cfg!(windows) {
        b"\x1b[113;60;0;1;0;1_\x1b[113;60;0;0;0;1_"
    } else {
        b"\x1bOQ"
    };
    write_input(writer, bytes);
}
