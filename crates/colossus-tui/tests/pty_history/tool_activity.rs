use super::*;

struct KillOnDrop(Box<dyn portable_pty::ChildKiller + Send + Sync>);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

pub(super) async fn emit_large_read(
    events: &mpsc::Sender<HostEvent>,
    session: &str,
) -> Result<(), String> {
    let call = ToolCall {
        call_id: "large-read".into(),
        name: "filesystem.read".into(),
        arguments: serde_json::json!({"path":"src/runtime.rs"}),
    };
    for event in [
        RunEvent::ToolStarted {
            turn: 1,
            call: call.clone(),
            elapsed_seconds: 0.4,
        },
        RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                images: vec![],
                call_id: call.call_id,
                name: call.name,
                output: format!(
                    "FILE-DETAIL-BEGIN\n{}FILE-DETAIL-TAIL",
                    "fn inspected_function() {}\n".repeat(10_000)
                ),
                exit_code: 0,
            },
            duration_seconds: 0.1,
            elapsed_seconds: 0.5,
        },
    ] {
        events
            .send(HostEvent::Run(RunEventEnvelope {
                schema_version: 1,
                run_id: "run-stream-pty".into(),
                session_id: session.into(),
                event,
            }))
            .await
            .map_err(|_| "fixture event receiver closed")?;
    }
    Ok(())
}

fn send(writer: &mut dyn std::io::Write, bytes: &[u8]) {
    writer.write_all(bytes).unwrap();
    writer.flush().unwrap();
}

fn toggle(writer: &mut dyn std::io::Write) {
    send(
        writer,
        if cfg!(windows) {
            b"\x1b[114;61;0;1;0;1_\x1b[114;61;0;0;0;1_"
        } else {
            b"\x1bOR"
        },
    );
}

fn wait_for_compact_frame(output: &Arc<Mutex<Vec<u8>>>, mode: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = screen_contents(output, 32, 120);
        let expanded_cleared = mode == "inline"
            || (!screen.contains("lines omitted") && !screen.contains("FILE-DETAIL-BEGIN"));
        if screen.contains("F3 expand")
            && screen.contains("▸ ✓ Read file")
            && screen.contains("ready")
            && screen.contains("preserved draft")
            && expanded_cleared
        {
            return screen;
        }
        assert!(
            Instant::now() < deadline,
            "PTY did not finish the compact {mode} frame:\n{screen}"
        );
        // Match the existing bounded screen wait; do not accept stale draft or
        // ready text as acknowledgement that the F3 redraw has completed.
        thread::sleep(Duration::from_millis(25));
    }
}

fn capture(output: &Arc<Mutex<Vec<u8>>>, name: &str) {
    if let Some(directory) = std::env::var_os("COLOSSUS_TUI_CAPTURE_DIR") {
        let path = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&path).unwrap();
        let raw = output.lock().unwrap().clone();
        std::fs::write(path.join(format!("{name}.ansi")), &raw).unwrap();
        let mut parser = vt100::Parser::new(32, 120, 5000);
        parser.process(&raw);
        std::fs::write(path.join(format!("{name}.txt")), parser.screen().contents()).unwrap();
        let mut cells = Vec::new();
        for row in 0..32 {
            for col in 0..120 {
                let cell = parser.screen().cell(row, col).unwrap();
                cells.push(serde_json::json!({"row":row,"col":col,"text":cell.contents(),
                    "fg":format!("{:?}",cell.fgcolor()),"bg":format!("{:?}",cell.bgcolor()),"bold":cell.bold()}));
            }
        }
        std::fs::write(
            path.join(format!("{name}.json")),
            serde_json::to_vec(&cells).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn f3_toggles_inline_tool_results_without_leaving_the_conversation() {
    for mode in ["alternate", "inline"] {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 32,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = fixture_command();
        command.args(["--exact", "fixture_process", "--nocapture"]);
        command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
        command.env("COLOSSUS_TUI_STREAM_FIXTURE", "1");
        command.env("COLOSSUS_TUI_TOOL_ACTIVITY_FIXTURE", "1");
        command.env("COLOSSUS_TUI_MODE", mode);
        let mut child = pair.slave.spawn_command(command).unwrap();
        let _cleanup = KillOnDrop(child.clone_killer());
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let reader_output = Arc::clone(&output);
        let thread = thread::spawn(move || {
            let mut bytes = [0; 8192];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => reader_output
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .extend_from_slice(&bytes[..read]),
                }
            }
        });
        let mut writer = pair.master.take_writer().unwrap();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if cfg!(windows) || mode == "inline" {
                wait_for_raw(&output, b"\x1b[6n");
                send(&mut writer, b"\x1b[1;1R");
            }
            wait_for_screen(&output, 32, 120, "Message · Enter sends");
            send(&mut writer, b"review repository\r");
            wait_for_screen(&output, 32, 120, "Repository review complete");
            assert!(!screen_contents(&output, 32, 120).contains("FILE-DETAIL-BEGIN"));
            if mode == "alternate" {
                assert!(screen_contents(&output, 32, 120).contains("Map repository"));
                assert!(screen_contents(&output, 32, 120).contains("Files: 200"));
            }
            wait_for_screen(&output, 32, 120, "ready");
            send(&mut writer, b"preserved draft");
            wait_for_screen(&output, 32, 120, "preserved draft");
            capture(&output, &format!("{mode}-compact"));
            toggle(&mut writer);
            if mode == "inline" {
                // Native scrollback is immutable. The new mode applies to the next turn,
                // without entering a transient alternate screen or duplicating history.
                send(&mut writer, &[3]);
                send(&mut writer, b"expanded repository\r");
            }
            wait_for_screen(&output, 32, 120, "lines omitted");
            wait_for_screen(&output, 32, 120, "ready");
            if mode == "inline" {
                send(&mut writer, b"preserved draft");
            } else {
                send(&mut writer, b"\x1b[5~");
                wait_for_screen(&output, 32, 120, "Duration");
            }
            wait_for_screen(&output, 32, 120, "preserved draft");
            let expanded = screen_contents(&output, 32, 120);
            assert!(expanded.contains("Message · Enter sends"), "{expanded}");
            assert!(!expanded.contains("Tool details"), "{expanded}");
            assert!(!expanded.contains("[ Close ]"), "{expanded}");
            assert!(!expanded.contains("json preview"), "{expanded}");
            capture(&output, &format!("{mode}-expanded-inline"));
            toggle(&mut writer);
            if mode == "inline" {
                send(&mut writer, &[3]);
                send(&mut writer, b"compact repository\r");
            }
            wait_for_screen(&output, 32, 120, "▸ ✓ Read file");
            wait_for_screen(&output, 32, 120, "ready");
            if mode == "inline" {
                send(&mut writer, b"preserved draft");
            }
            wait_for_screen(&output, 32, 120, "preserved draft");
            let compact = wait_for_compact_frame(&output, mode);
            assert!(!compact.contains("Tool details"), "{compact}");
            if mode == "alternate" {
                assert!(!compact.contains("lines omitted"), "{compact}");
                assert!(!compact.contains("FILE-DETAIL-BEGIN"), "{compact}");
            }
            capture(&output, &format!("{mode}-compact-restored"));
            send(&mut writer, &[3]);
            thread::sleep(Duration::from_millis(60));
            send(&mut writer, &[3]);
        }));
        if outcome.is_err() {
            let _ = child.kill();
        }
        let status = child.wait().unwrap();
        drop(writer);
        drop(pair.master);
        thread.join().unwrap();
        if let Err(failure) = outcome {
            std::panic::resume_unwind(failure);
        }
        assert!(status.success());
    }
}
