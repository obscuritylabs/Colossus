//! Explicit real-desktop acceptance against nonce-named synthetic sibling windows.
#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Some(code) = colossus_native_computer::run_if_requested() {
        std::process::exit(if code == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        })
    }
    if let Err(error) = run().await {
        eprintln!("native computer acceptance failed: {error}");
        std::process::exit(1)
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("native Windows acceptance requires an interactive ordinary-user desktop");
    std::process::exit(78)
}

#[cfg(windows)]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut title = None;
    let mut output = None;
    let mut require_cover = false;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fixture-title" => title = args.next(),
            "--output" => output = args.next(),
            "--require-cover" => require_cover = true,
            _ => return Err("unknown acceptance argument".into()),
        }
    }
    let title = title
        .filter(|value| value.starts_with("Colossus computer fixture ") && value.len() == 58)
        .ok_or("missing exact nonce fixture title")?;
    let output =
        std::path::PathBuf::from(output.ok_or("missing private artifact output directory")?);
    if !output.is_absolute() || !output.is_dir() {
        return Err("output must be an existing absolute private directory".into());
    }
    let evidence = Evidence {
        output: output.clone(),
        stage: std::cell::Cell::new("construct_driver"),
        finished: std::cell::Cell::new(false),
    };
    let result = acceptance(title, output, require_cover, &evidence).await;
    evidence.finish(result.as_ref().err().map(|e| e.to_string()))?;
    result
}

#[cfg(windows)]
struct Evidence {
    output: std::path::PathBuf,
    stage: std::cell::Cell<&'static str>,
    finished: std::cell::Cell<bool>,
}
#[cfg(windows)]
impl Evidence {
    fn set(&self, stage: &'static str) {
        self.stage.set(stage);
        println!("STAGE {stage}");
    }
    fn finish(&self, error: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(error) = error {
            let error = error.chars().take(1024).collect::<String>();
            let record = serde_json::to_vec(
                &serde_json::json!({"schema_version":1,"passed":false,"stage":self.stage.get(),"error":error}),
            )?;
            colossus_windows_native::create_private_file(
                &self.output.join("failure.json"),
                &record,
            )?;
        }
        self.finished.set(true);
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for Evidence {
    fn drop(&mut self) {
        if !self.finished.get() {
            let _ = self.finish(Some("acceptance did not reach a terminal result".into()));
        }
    }
}

#[cfg(windows)]
async fn acceptance(
    title: String,
    output: std::path::PathBuf,
    require_cover: bool,
    evidence: &Evidence,
) -> Result<(), Box<dyn std::error::Error>> {
    use colossus_contracts::*;
    use colossus_native_computer::{NativeComputerDriver, TrustedComputerContext};
    use colossus_ports::*;
    use sha2::{Digest, Sha256};
    if require_cover {
        evidence.set("wait_for_cover_foreground");
        let cover = title
            .replace("fixture", "cover")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            // SAFETY: read-only exact synthetic title/foreground queries. No
            // native helper, session, hooks or input exist during setup.
            let ready = unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::*;
                let hwnd = FindWindowW(std::ptr::null(), cover.as_ptr());
                !hwnd.is_null() && GetForegroundWindow() == hwnd
            };
            if ready {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("exact synthetic cover must be activated for foreground setup".into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
    evidence.set("construct_driver");
    let driver = NativeComputerDriver::new(
        std::env::current_exe()?,
        TrustedComputerContext::for_current_host()?,
    )?;
    let binding = ComputerBinding {
        runtime_id: "native-acceptance".into(),
        workspace_id: "synthetic".into(),
        application_id: "native-acceptance".into(),
        scope: ComputerScope::Conversation {
            id: "fixture".into(),
        },
        run_id: format!("acceptance-{}", std::process::id()),
    };
    let control = RunControl::default();
    macro_rules! dispatch {
        ($command:expr) => {{
            let command = $command;
            evidence.set(command_stage(&command));
            driver
                .execute(
                    ComputerDriverRequest {
                        binding: binding.clone(),
                        command,
                    },
                    &control,
                )
                .await?
        }};
    }
    let result = async {
        let ComputerDriverReply::Windows(windows) =
            dispatch!(ComputerCommand::Windows { max_windows: 128 })
        else {
            return Err("missing native window list".into());
        };
        let window = windows.iter().find(|w| w.title == title).ok_or(
            "synthetic sibling target not available (medium-user interactive desktop required)",
        )?;
        if require_cover
            && !windows
                .iter()
                .any(|w| w.title == title.replace("fixture", "cover") && w.foreground)
        {
            return Err("synthetic foreign cover must initially be foreground".into());
        }
        let ComputerDriverReply::Opened(session) = dispatch!(ComputerCommand::Open {
            window_id: window.window_id.clone(),
            limits: ComputerLimits::default()
        }) else {
            return Err("missing opened session".into());
        };
        // A second independently supervised run may inspect choices but cannot
        // become a second writer for this Windows logon session.
        evidence.set("writer_contention");
        writer_contention(&driver, &binding, &title).await?;
        let ComputerDriverReply::Screenshot(capture) = dispatch!(ComputerCommand::Screenshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_bytes: 8 * 1024 * 1024
        }) else {
            return Err("missing WGC PNG".into());
        };
        let image =
            image::load_from_memory_with_format(&capture.png, image::ImageFormat::Png)?.to_rgba8();
        if image.dimensions() != (capture.descriptor.width, capture.descriptor.height)
            || format!("{:x}", Sha256::digest(&capture.png)) != capture.descriptor.sha256
        {
            return Err("native PNG descriptor mismatch".into());
        }
        let pixel = image.get_pixel(image.width() / 2, image.height() / 2).0;
        if pixel[..3] != [27, 154, 89] {
            return Err(
                "WGC did not capture selected fixture pixels through foreign occlusion".into(),
            );
        }
        colossus_windows_native::create_private_file(
            &output.join("selected-window.png"),
            &capture.png,
        )?;
        println!("PASS target-only WGC PNG through foreign occlusion");
        if capture.root_window_id != session.window_id {
            return Err("screenshot lost selected root provenance".into());
        }
        if let Some(cover) = windows
            .iter()
            .find(|w| w.title == title.replace("fixture", "cover"))
        {
            let denied = driver
                .execute(
                    ComputerDriverRequest {
                        binding: binding.clone(),
                        command: ComputerCommand::Input {
                            session_id: session.session_id.clone(),
                            control_generation: session.control_generation,
                            observation_id: capture.descriptor.observation_id.clone(),
                            input: ComputerInput::Focus {
                                window_id: cover.window_id.clone(),
                            },
                        },
                    },
                    &control,
                )
                .await;
            if denied != Err(ComputerDriverError::Denied) {
                return Err("foreign window admitted into selected session".into());
            }
        }
        if require_cover {
            // Observation does not activate the target. Refreshing a background
            // snapshot must never turn it into authority to click through cover.
            let foreground_diagnostic = FixtureForegroundDiagnostic::new(&title)?;
            for stage in [
                "background_click_without_focus",
                "background_click_after_fresh_snapshot",
            ] {
                let ComputerDriverReply::Snapshot(observation) =
                    dispatch!(ComputerCommand::Snapshot {
                        session_id: session.session_id.clone(),
                        control_generation: session.control_generation,
                        max_nodes: 512
                    })
                else {
                    return Err("missing background native UIA tree".into());
                };
                if !observation
                    .windows
                    .iter()
                    .any(|observed| observed.window_id == session.window_id && !observed.foreground)
                    || !observation
                        .nodes
                        .iter()
                        .any(|node| node.name == "Waiting for action")
                {
                    return Err("background click setup changed or applied an action".into());
                }
                let button = observation
                    .nodes
                    .iter()
                    .find(|node| node.name == "Fixture action")
                    .ok_or("background native button missing from UIA")?;
                evidence.set(stage);
                let foreground_before = foreground_diagnostic.sample();
                let denied = driver
                    .execute(
                        ComputerDriverRequest {
                            binding: binding.clone(),
                            command: ComputerCommand::Input {
                                session_id: session.session_id.clone(),
                                control_generation: session.control_generation,
                                observation_id: observation.observation_id,
                                input: ComputerInput::Click {
                                    target: ComputerTarget::Element {
                                        element_id: button.element_id.clone(),
                                    },
                                    button: ComputerMouseButton::Left,
                                    count: 1,
                                },
                            },
                        },
                        &control,
                    )
                    .await;
                let foreground_after = foreground_diagnostic.sample();
                if denied != Err(ComputerDriverError::Stale) {
                    let result = match &denied {
                        Err(error) => serde_json::json!({"kind":"error","category":error}),
                        Ok(reply) => serde_json::json!({
                            "kind":"reply",
                            "category":match reply {
                                ComputerDriverReply::Windows(_) => "windows",
                                ComputerDriverReply::Opened(_) => "opened",
                                ComputerDriverReply::Status(_) => "status",
                                ComputerDriverReply::Snapshot(_) => "snapshot",
                                ComputerDriverReply::Screenshot(_) => "screenshot",
                                ComputerDriverReply::Applied(_) => "applied",
                                ComputerDriverReply::Closed => "closed",
                            }
                        }),
                    };
                    let record = serde_json::to_vec(&serde_json::json!({
                        "schema_version":1,"passed":false,"stage":stage,
                        "expected_error":"stale","actual_result":result,
                        "observation_foreground":false,"observation_waiting_for_action":true,
                        "foreground_before":foreground_before,"foreground_after":foreground_after,
                        "input_attempts":1
                    }))?;
                    if record.len() > 1024 {
                        return Err("background input diagnostic exceeded its bound".into());
                    }
                    eprintln!(
                        "COMPUTER_BACKGROUND_INPUT stage={stage} result={result} before={foreground_before} after={foreground_after}"
                    );
                    colossus_windows_native::create_private_file(
                        &evidence.output.join("background-input-diagnostic.json"),
                        &record,
                    )?;
                    return Err("background click did not fail as a known no-effect Stale".into());
                }
            }
            let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
                session_id: session.session_id.clone(),
                control_generation: session.control_generation,
                max_nodes: 512
            }) else {
                return Err("missing background denial result".into());
            };
            if !observation
                .windows
                .iter()
                .any(|observed| observed.window_id == session.window_id && !observed.foreground)
                || !observation
                    .nodes
                    .iter()
                    .any(|node| node.name == "Waiting for action")
            {
                return Err("denied background click changed focus or reached the fixture".into());
            }
            println!("PASS background clicks remain no-effect Stale after fresh snapshots");
        }
        let ComputerDriverReply::Screenshot(capture) = dispatch!(ComputerCommand::Screenshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_bytes: 8 * 1024 * 1024
        }) else {
            return Err("missing fresh focus observation".into());
        };
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: capture.descriptor.observation_id.clone(),
            input: ComputerInput::Focus {
                window_id: window.window_id.clone()
            }
        });
        let replay = driver
            .execute(
                ComputerDriverRequest {
                    binding: binding.clone(),
                    command: ComputerCommand::Input {
                        session_id: session.session_id.clone(),
                        control_generation: session.control_generation,
                        observation_id: capture.descriptor.observation_id,
                        input: ComputerInput::Move { x: 20, y: 20 },
                    },
                },
                &control,
            )
            .await;
        if replay != Err(ComputerDriverError::Stale) {
            return Err("consumed observation was reusable".into());
        }
        let ComputerDriverReply::Snapshot(pointer_observation) =
            dispatch!(ComputerCommand::Snapshot {
                session_id: session.session_id.clone(),
                control_generation: session.control_generation,
                max_nodes: 512
            })
        else {
            return Err("missing pointer-motion observation".into());
        };
        evidence.set("foreign_pointer_motion_retains_control");
        fixture_pointer_motion(&title)?;
        let ComputerDriverReply::Status(pointer_status) = dispatch!(ComputerCommand::Status {
            session_id: session.session_id.clone()
        }) else {
            return Err("missing pointer-motion control state".into());
        };
        if pointer_status.state != ComputerControlState::Active
            || pointer_status.control_generation != session.control_generation
            || pointer_status.reason.is_some()
        {
            return Err("foreign pointer movement interrupted native control".into());
        }
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: pointer_observation.observation_id,
            input: ComputerInput::Move { x: 20, y: 20 }
        });
        println!(
            "PASS foreign pointer motion preserves control and the next native input succeeds"
        );
        let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_nodes: 512
        }) else {
            return Err("missing native UIA tree".into());
        };
        let button = observation
            .nodes
            .iter()
            .find(|node| node.name == "Fixture action")
            .ok_or("native button missing from UIA")?;
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: observation.observation_id,
            input: ComputerInput::Click {
                target: ComputerTarget::Element {
                    element_id: button.element_id.clone()
                },
                button: ComputerMouseButton::Left,
                count: 1
            }
        });
        let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_nodes: 512
        }) else {
            return Err("missing post-click tree".into());
        };
        if !observation.nodes.iter().any(|node| node.name == "Clicked") {
            return Err("native click did not reach synthetic app".into());
        }
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: observation.observation_id,
            input: ComputerInput::Type {
                text: "Colossus 😀 native input".into()
            }
        });
        let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_nodes: 512
        }) else {
            return Err("missing pre-chord tree".into());
        };
        if !observation
            .nodes
            .iter()
            .any(|node| node.name == "Typed Colossus 😀 native input")
        {
            return Err("Unicode text did not reach native fixture".into());
        }
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: observation.observation_id,
            input: ComputerInput::Press {
                keys: vec!["Control".into(), "A".into()]
            }
        });
        let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_nodes: 512
        }) else {
            return Err("missing pre-drag tree".into());
        };
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: observation.observation_id,
            input: ComputerInput::Drag {
                from_x: 350,
                from_y: 260,
                to_x: 450,
                to_y: 300,
                steps: 16
            }
        });
        let observation =
            wait_for_drag_postcondition(&driver, &binding, &session, &control, &title, evidence)
                .await?;
        let modal = observation
            .nodes
            .iter()
            .find(|n| n.name == "Open owned modal")
            .ok_or("missing modal control")?;
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: observation.observation_id,
            input: ComputerInput::Click {
                target: ComputerTarget::Element {
                    element_id: modal.element_id.clone()
                },
                button: ComputerMouseButton::Left,
                count: 1
            }
        });
        // Screenshot is the first observation after opening the modal. Permit
        // native event delivery latency, but never insert a snapshot to seed a
        // modal allowlist before checking its private root provenance.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let ComputerDriverReply::Screenshot(modal) = dispatch!(ComputerCommand::Screenshot {
                session_id: session.session_id.clone(),
                control_generation: session.control_generation,
                max_bytes: 8 * 1024 * 1024
            }) else {
                return Err("missing immediate owned-modal screenshot".into());
            };
            if modal.root_window_id != session.window_id {
                return Err("owned modal screenshot root mismatch".into());
            }
            if modal.descriptor.window_id != session.window_id {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("owned modal never selected for screenshot".into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_nodes: 512
        }) else {
            return Err("missing owned-modal UIA tree".into());
        };
        let close = observation
            .nodes
            .iter()
            .find(|n| n.name == "Close modal")
            .ok_or("missing owned-modal close control")?;
        evidence.set("owned_modal_reparent_denial");
        fixture_modal_ownership(&title, true)?;
        let denied = driver
            .execute(
                ComputerDriverRequest {
                    binding: binding.clone(),
                    command: ComputerCommand::Input {
                        session_id: session.session_id.clone(),
                        control_generation: session.control_generation,
                        observation_id: observation.observation_id.clone(),
                        input: ComputerInput::Focus {
                            window_id: observation.window_id.clone(),
                        },
                    },
                },
                &control,
            )
            .await;
        // Always restore this disposable target even when the native fence fails.
        fixture_modal_ownership(&title, false)?;
        if denied != Err(ComputerDriverError::Denied) {
            return Err("a detached modal retained selected-root input authority".into());
        }
        let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_nodes: 512
        }) else {
            return Err("missing restored-modal observation".into());
        };
        let close = observation
            .nodes
            .iter()
            .find(|n| n.name == close.name)
            .ok_or("restored modal close missing")?;
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: observation.observation_id,
            input: ComputerInput::Click {
                target: ComputerTarget::Element {
                    element_id: close.element_id.clone()
                },
                button: ComputerMouseButton::Left,
                count: 1
            }
        });
        println!("PASS same-app owned modal input and immediate screenshot provenance");
        control.cancel();
        driver.revoke_run(&binding);
        let denied = driver
            .execute(
                ComputerDriverRequest {
                    binding: binding.clone(),
                    command: ComputerCommand::Windows { max_windows: 1 },
                },
                &control,
            )
            .await;
        if denied != Err(ComputerDriverError::Denied) {
            return Err("revoked run regained authority".into());
        }
        println!("PASS native UIA, click, Unicode input, chord, balanced drag and revocation");
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let failed_stage = evidence.stage.get();
    evidence.set("main_helper_cleanup");
    let cleanup = driver.close_run(&binding).await;
    if result.is_err() {
        evidence.stage.set(failed_stage);
    }
    result?;
    cleanup?;
    evidence.set("helper_exit_drag");
    helper_exit_acceptance(&driver, &binding, &title).await?;
    evidence.set("publish_receipt");
    colossus_windows_native::create_private_file(
        &output.join("acceptance.json"),
        &serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"passed":true,"owned_modal":true,"modal_reparent_denied":true,
            "writer_contention":true,"foreign_window_denied":true,"cancellation":true,
            "balanced_drag_helper_exit":true,"background_focus_regression_checked":require_cover,
            "foreign_pointer_motion_continued":true
        }))?,
    )?;
    println!("PASS native helper acknowledged cleanup; synthetic user apps remain running");
    Ok(())
}

#[cfg(windows)]
async fn wait_for_drag_postcondition(
    driver: &colossus_native_computer::NativeComputerDriver,
    binding: &colossus_contracts::ComputerBinding,
    session: &colossus_contracts::ComputerSessionSummary,
    control: &colossus_ports::RunControl,
    title: &str,
    evidence: &Evidence,
) -> Result<colossus_contracts::ComputerObservation, Box<dyn std::error::Error>> {
    use colossus_ports::{
        ComputerCommand, ComputerDriver as _, ComputerDriverReply, ComputerDriverRequest,
    };
    use std::time::{Duration, Instant};
    // Balanced insertion is not a foreign WndProc acknowledgement. Observe the
    // one already-sent gesture; never focus, resume or send it again.
    let started = Instant::now();
    let mut attempts = 0;
    let mut first = None;
    let mut last = None;
    let mut immediate_matched = false;
    let mut final_matched = false;
    let result: Result<_, Box<dyn std::error::Error>> = loop {
        attempts += 1;
        evidence.set("drag_postcondition_snapshot");
        let reply = driver
            .execute(
                ComputerDriverRequest {
                    binding: binding.clone(),
                    command: ComputerCommand::Snapshot {
                        session_id: session.session_id.clone(),
                        control_generation: session.control_generation,
                        max_nodes: 512,
                    },
                },
                control,
            )
            .await;
        let observation = match reply {
            Ok(ComputerDriverReply::Snapshot(observation)) => observation,
            Ok(_) => break Err("missing drag result".into()),
            Err(error) => break Err(error.into()),
        };
        let same_target = observation.session_id == session.session_id
            && observation.control_generation == session.control_generation
            && observation.window_id == session.window_id
            && observation.windows.iter().any(|window| {
                window.window_id == session.window_id && window.title == title && window.foreground
            });
        let matched = observation
            .nodes
            .iter()
            .any(|node| node.name == "Drag received");
        let mut sample = drag_observation_sample(&observation, title, started.elapsed(), matched);
        sample["same_target"] = serde_json::json!(same_target);
        if first.is_none() {
            first = Some(sample.clone());
            immediate_matched = matched;
        }
        last = Some(sample);
        final_matched = matched;
        if !same_target {
            break Err("drag postcondition target or control generation changed".into());
        }
        if matched {
            break Ok(observation);
        }
        if attempts >= 8 || started.elapsed() >= Duration::from_secs(3) {
            break Err("balanced native drag did not reach fixture".into());
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    evidence.set("drag_postcondition");
    let diagnostic = serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"passed":result.is_ok(),"attempts":attempts,
        "elapsed_ms":started.elapsed().as_millis(),"immediate_matched":immediate_matched,
        "final_matched":final_matched,"first_observation":first,"last_observation":last,
        "input_attempts":1,"maximum_observations":8,"observation_budget_ms":3000,
        "expected_session_id":session.session_id,"expected_generation":session.control_generation,
        "expected_root_window_id":session.window_id,
        "error":result.as_ref().err().map(|_| "drag_postcondition_not_established")
    }))?;
    if diagnostic.len() > 64 * 1024 {
        return Err("drag diagnostic exceeded its bound".into());
    }
    colossus_windows_native::create_private_file(
        &evidence.output.join("drag-postcondition.json"),
        &diagnostic,
    )?;
    result
}

// Read-only diagnostic classification of the two nonce fixture handles. These
// handles never admit a target or grant capture, focus, or input authority.
#[cfg(windows)]
struct FixtureForegroundDiagnostic {
    fixture: (windows_sys::Win32::Foundation::HWND, u32, u32),
    cover: (windows_sys::Win32::Foundation::HWND, u32, u32),
}

#[cfg(windows)]
impl FixtureForegroundDiagnostic {
    fn new(title: &str) -> Result<Self, Box<dyn std::error::Error>> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
        let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let class = wide("ColossusDisposableComputerFixture");
        let find = |title: &str| {
            let title = wide(title);
            let mut process = 0;
            // SAFETY: exact synthetic class and nonce title only; read-only
            // queries freeze handles and their process/thread identities.
            let (window, thread) = unsafe {
                let window = FindWindowW(class.as_ptr(), title.as_ptr());
                (window, GetWindowThreadProcessId(window, &mut process))
            };
            if window.is_null() || process == 0 || thread == 0 {
                return Err("owned foreground diagnostic window is unavailable");
            }
            Ok((window, process, thread))
        };
        Ok(Self {
            fixture: find(title)?,
            cover: find(&title.replace("fixture", "cover"))?,
        })
    }

    fn sample(&self) -> &'static str {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        // SAFETY: read-only foreground handle comparison. Process/thread
        // metadata is queried only for an exact frozen synthetic handle; no
        // foreign title, process metadata, or raw handle is retained or logged.
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground.is_null() {
                return "none";
            }
            for (category, (window, process, thread)) in
                [("fixture", self.fixture), ("cover", self.cover)]
            {
                if foreground == window {
                    let mut current_process = 0;
                    let current_thread = GetWindowThreadProcessId(window, &mut current_process);
                    if current_process == process && current_thread == thread {
                        return category;
                    }
                }
            }
            "other"
        }
    }
}

#[cfg(windows)]
fn drag_observation_sample(
    observation: &colossus_contracts::ComputerObservation,
    title: &str,
    elapsed: std::time::Duration,
    matched: bool,
) -> serde_json::Value {
    use sha2::{Digest as _, Sha256};
    let labels = [
        "",
        "Fixture input",
        "Fixture action",
        "Open owned modal",
        "Waiting for action",
        "Clicked",
        "Typed Colossus 😀 native input",
        "Drag received",
    ];
    let nodes = observation
        .nodes
        .iter()
        .take(16)
        .map(|node| {
            let synthetic = node.name == title || labels.contains(&node.name.as_str());
            serde_json::json!({
                "name":synthetic.then_some(&node.name),"name_bytes":node.name.len(),
                "name_sha256":format!("{:x}",Sha256::digest(node.name.as_bytes())),
                "role":synthetic.then_some(&node.role),"bounds":node.bounds,
                "focused":node.focused,"is_password":node.is_password
            })
        })
        .collect::<Vec<_>>();
    // SAFETY: read-only shared monotonic system tick for correlating owned
    // fixture diagnostics; no window, input or process operation is performed.
    let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() };
    serde_json::json!({
        "elapsed_ms":elapsed.as_millis(),"system_tick_ms":tick,"matched":matched,
        "session_id":observation.session_id,"generation":observation.control_generation,
        "window_id":observation.window_id,"width":observation.width,"height":observation.height,
        "scale_milli":observation.scale_milli,"node_count":observation.nodes.len(),"nodes":nodes,
        "foreground":observation.windows.iter().find(|window| window.window_id == observation.window_id).map(|window| window.foreground)
    })
}

#[cfg(windows)]
fn fixture_pointer_motion(title: &str) -> Result<(), Box<dyn std::error::Error>> {
    use windows_sys::Win32::{
        Foundation::RECT,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    };
    let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut bounds = RECT::default();
    // SAFETY: exact nonce fixture/class and foreground queries; no personal app
    // is admitted. This acceptance-only input is pointer motion with no buttons.
    let (root, width, height, origin_x, origin_y) = unsafe {
        let root = FindWindowW(
            wide("ColossusDisposableComputerFixture").as_ptr(),
            wide(title).as_ptr(),
        );
        if root.is_null() || GetForegroundWindow() != root || GetWindowRect(root, &mut bounds) == 0
        {
            return Err(
                "pointer regression requires the exact foreground synthetic fixture".into(),
            );
        }
        (
            root,
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        )
    };
    if width <= 1 || height <= 1 {
        return Err("pointer regression requires bounded virtual desktop geometry".into());
    }
    let center = |near: i32, far: i32| (i64::from(near) + i64::from(far)) / 2;
    let normalize =
        |coordinate: i64, origin: i32, size: i32| -> Result<i32, Box<dyn std::error::Error>> {
            let offset = coordinate - i64::from(origin);
            if !(0..i64::from(size)).contains(&offset) {
                return Err("synthetic pointer target lies outside the virtual desktop".into());
            }
            Ok(i32::try_from(offset * 65535 / i64::from(size - 1))?)
        };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: normalize(center(bounds.left, bounds.right), origin_x, width)?,
                dy: normalize(center(bounds.top, bounds.bottom), origin_y, height)?,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: one fully initialized mouse-motion event. Recheck the exact
    // synthetic foreground before sending; no click or key can be generated.
    if unsafe {
        GetForegroundWindow() != root
            || SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) != 1
    } {
        return Err("foreign synthetic pointer motion was not accepted".into());
    }
    Ok(())
}

#[cfg(windows)]
fn command_stage(command: &colossus_ports::ComputerCommand) -> &'static str {
    use colossus_ports::ComputerCommand;
    match command {
        ComputerCommand::Windows { .. } => "windows",
        ComputerCommand::Open { .. } => "open",
        ComputerCommand::Status { .. } => "status",
        ComputerCommand::Snapshot { .. } => "snapshot",
        ComputerCommand::Screenshot { .. } => "screenshot",
        ComputerCommand::Input { input, .. } => match input {
            colossus_contracts::ComputerInput::Focus { .. } => "focus",
            colossus_contracts::ComputerInput::Move { .. } => "move",
            colossus_contracts::ComputerInput::Click { .. } => "click",
            colossus_contracts::ComputerInput::Drag { .. } => "drag",
            colossus_contracts::ComputerInput::Scroll { .. } => "scroll",
            colossus_contracts::ComputerInput::Type { .. } => "type",
            colossus_contracts::ComputerInput::Press { .. } => "press",
        },
        ComputerCommand::Close { .. } => "close",
    }
}

#[cfg(windows)]
fn fixture_modal_ownership(title: &str, detach: bool) -> Result<(), Box<dyn std::error::Error>> {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    // SAFETY: test-only mutation is bounded to the nonce root's exact process,
    // fixed fixture class and fixed synthetic modal; no model/native API uses it.
    unsafe {
        let root = FindWindowW(std::ptr::null(), wide(title).as_ptr());
        let modal = FindWindowW(
            wide("ColossusDisposableComputerFixture").as_ptr(),
            wide("Owned computer fixture modal").as_ptr(),
        );
        let (mut root_pid, mut modal_pid) = (0, 0);
        GetWindowThreadProcessId(root, &mut root_pid);
        GetWindowThreadProcessId(modal, &mut modal_pid);
        if root.is_null()
            || modal.is_null()
            || root_pid == 0
            || root_pid != modal_pid
            || (detach && GetWindow(modal, GW_OWNER) != root)
        {
            return Err("private fixture ownership target did not match nonce root".into());
        }
        let mut receipt = 0;
        if SendMessageTimeoutW(
            modal,
            WM_APP + if detach { 71 } else { 72 },
            0,
            0,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            1000,
            &mut receipt,
        ) == 0
            || receipt != 1
        {
            return Err("private fixture ownership operation did not acknowledge".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn writer_contention(
    driver: &colossus_native_computer::NativeComputerDriver,
    base: &colossus_contracts::ComputerBinding,
    title: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use colossus_contracts::*;
    use colossus_ports::*;
    let mut binding = base.clone();
    binding.run_id.push_str("-competing");
    let control = RunControl::default();
    let result = async {
        let ComputerDriverReply::Windows(windows) = driver
            .execute(
                ComputerDriverRequest {
                    binding: binding.clone(),
                    command: ComputerCommand::Windows { max_windows: 128 },
                },
                &control,
            )
            .await?
        else {
            return Err("missing competing run window list".into());
        };
        let window = windows
            .iter()
            .find(|w| w.title == title)
            .ok_or("missing competing target")?;
        let result = driver
            .execute(
                ComputerDriverRequest {
                    binding: binding.clone(),
                    command: ComputerCommand::Open {
                        window_id: window.window_id.clone(),
                        limits: ComputerLimits::default(),
                    },
                },
                &control,
            )
            .await;
        if result != Err(ComputerDriverError::Busy) {
            return Err("two native computer writers acquired the same logon lease".into());
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    control.cancel();
    driver.revoke_run(&binding);
    let cleanup = driver.close_run(&binding).await;
    result?;
    cleanup?;
    println!("PASS cross-process single-writer admission");
    Ok(())
}

#[cfg(windows)]
async fn helper_exit_acceptance(
    driver: &colossus_native_computer::NativeComputerDriver,
    base: &colossus_contracts::ComputerBinding,
    title: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use colossus_contracts::*;
    use colossus_ports::*;
    let mut binding = base.clone();
    binding.run_id.push_str("-helper-exit");
    let control = RunControl::default();
    macro_rules! dispatch {
        ($command:expr) => {
            driver
                .execute(
                    ComputerDriverRequest {
                        binding: binding.clone(),
                        command: $command,
                    },
                    &control,
                )
                .await?
        };
    }
    let result = async {
        let ComputerDriverReply::Windows(windows) =
            dispatch!(ComputerCommand::Windows { max_windows: 128 })
        else {
            return Err("missing hard-exit window list".into());
        };
        let window = windows
            .iter()
            .find(|w| w.title == title)
            .ok_or("target app disappeared after session cleanup")?;
        let ComputerDriverReply::Opened(session) = dispatch!(ComputerCommand::Open {
            window_id: window.window_id.clone(),
            limits: ComputerLimits::default()
        }) else {
            return Err("missing hard-exit session".into());
        };
        let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_nodes: 512
        }) else {
            return Err("missing hard-exit observation".into());
        };
        dispatch!(ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            observation_id: observation.observation_id,
            input: ComputerInput::Focus {
                window_id: window.window_id.clone()
            }
        });
        let ComputerDriverReply::Snapshot(observation) = dispatch!(ComputerCommand::Snapshot {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
            max_nodes: 512
        }) else {
            return Err("missing pre-abort observation".into());
        };
        let request = ComputerDriverRequest {
            binding: binding.clone(),
            command: ComputerCommand::Input {
                session_id: session.session_id,
                control_generation: session.control_generation,
                observation_id: observation.observation_id,
                input: ComputerInput::Drag {
                    from_x: 350,
                    from_y: 260,
                    to_x: 450,
                    to_y: 300,
                    steps: 100,
                },
            },
        };
        let (mutation, abort) = tokio::join!(driver.execute(request, &control), async {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            driver.acceptance_abort_helper(&binding)
        });
        abort?;
        if !matches!(
            mutation,
            Ok(ComputerDriverReply::Applied(_))
                | Err(ComputerDriverError::OutcomeUnknown
                    | ComputerDriverError::Denied
                    | ComputerDriverError::Unavailable)
        ) {
            return Err("unexpected helper-exit mutation outcome".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        for vk in [
            VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN,
        ] {
            // SAFETY: read-only global key-state query; never releases human input.
            if (unsafe { GetAsyncKeyState(i32::from(vk)) }) < 0 {
                return Err(
                    "synthetic input remained held after helper death (or human input interfered)"
                        .into(),
                );
            }
        }
        let title16 = title.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        // SAFETY: read-only exact nonce title check, never acts on the user app.
        if unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(
                std::ptr::null(),
                title16.as_ptr(),
            )
        }
        .is_null()
        {
            return Err("helper exit closed the synthetic user app".into());
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    control.cancel();
    driver.revoke_run(&binding);
    let cleanup = driver.close_run(&binding).await;
    result?;
    cleanup?;
    println!(
        "PASS hard helper exit around balanced drag, released input, target survives and cleanup acknowledged"
    );
    Ok(())
}
