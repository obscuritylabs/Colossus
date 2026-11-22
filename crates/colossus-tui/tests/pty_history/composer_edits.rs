use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) const EXPECTED_PROMPT_ENV: &str = "COLOSSUS_TUI_COMPOSER_EDIT_EXPECTED_PROMPT";
const SUBMISSION_RECEIPT: &str = "Host received the exact edited draft";
static SUBMISSIONS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn accept_submission(
    request: InteractiveRunRequest,
    expected: &str,
) -> Result<HostRunResult, String> {
    let previous = SUBMISSIONS.fetch_add(1, Ordering::Relaxed);
    if previous != 0 || request.prompt != expected {
        return Err("Host received unexpected or repeated composer input".into());
    }
    Ok(HostRunResult {
        outcome: AgentRunOutcome::Completed {
            result: AgentRunResult {
                run_id: "run-composer-edit".into(),
                session_id: Some(request.session_id),
                role: "primary".into(),
                profile: "fixture".into(),
                model_profile: "fixture".into(),
                provider_profile: "local".into(),
                model: "fixture".into(),
                plan: None,
                output: SUBMISSION_RECEIPT.into(),
                event_count: 0,
                elapsed_seconds: 0.0,
            },
        },
        footer: FooterState {
            role: "primary".into(),
            route: "fixture@local".into(),
            status: "ready".into(),
            approval_mode: "ask".into(),
            ..Default::default()
        },
        plan_selection: PlanSelectionUpdate::Unchanged,
    })
}

pub(super) fn assert_single_submission() {
    assert_eq!(SUBMISSIONS.load(Ordering::Relaxed), 1, "host submissions");
}

#[test]
fn composer_undo_redo_shortcuts_edit_the_draft_before_one_exact_submission() {
    const ROWS: u16 = 24;
    const COLS: u16 = 100;
    const ORIGINAL: &str = "edit proof";
    const WITH_SUFFIX: &str = "edit proofx";
    const FINAL: &str = "edit proof!";

    #[cfg(unix)]
    let paste = format!("{}tail", "paste line\n".repeat(100));

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("composer edit PTY");
    let mut command = fixture_command();
    command.args(["--exact", "fixture_process", "--nocapture"]);
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    command.env("COLOSSUS_TUI_MODE", "alternate");
    #[cfg(unix)]
    command.env(EXPECTED_PROMPT_ENV, format!("{FINAL}{paste}"));
    #[cfg(not(unix))]
    command.env(EXPECTED_PROMPT_ENV, FINAL);
    let mut child = pair.slave.spawn_command(command).expect("composer fixture");
    let _child_cleanup = FixtureChildGuard(child.clone_killer());
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("PTY reader");
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut bytes = [0_u8; 8192];
        while let Ok(count) = reader.read(&mut bytes) {
            if count == 0 {
                break;
            }
            captured
                .lock()
                .expect("output")
                .extend_from_slice(&bytes[..count]);
        }
    });
    let mut writer = pair.master.take_writer().expect("PTY writer");
    #[cfg(windows)]
    answer_cursor_position_query(&output, &mut writer);
    wait_for_screen(&output, ROWS, COLS, "Message · Enter sends");

    let wait_for_draft = |draft: &str| {
        // Include the composer border and following blank so a longer draft
        // (or a transcript echo of the prompt) cannot satisfy this assertion.
        wait_for_screen(&output, ROWS, COLS, &format!("│{draft} "));
        assert!(!screen_contents(&output, ROWS, COLS).contains(SUBMISSION_RECEIPT));
    };
    for (input, expected) in [
        (ORIGINAL.as_bytes(), ORIGINAL),
        (b"x", WITH_SUFFIX),
        (CTRL_Z, ORIGINAL),
        (CTRL_Y, WITH_SUFFIX),
        (F7, ORIGINAL),
        (F8, WITH_SUFFIX),
        (CTRL_Z, ORIGINAL),
        (b"!", FINAL),
    ] {
        write_input(&mut writer, input);
        wait_for_draft(expected);
    }

    #[cfg(unix)]
    {
        // The Windows console backend does not emit bracketed-paste events.
        // On Unix the payload must reach the host, not the visible marker.
        let visible = format!("{FINAL}[Pasted Content {} chars]", paste.chars().count());
        let bracketed = format!("\x1b[200~{paste}\x1b[201~");
        write_input(&mut writer, bracketed.as_bytes());
        wait_for_draft(&visible);
        write_input(&mut writer, F7);
        wait_for_draft(FINAL);
        write_input(&mut writer, F8);
        wait_for_draft(&visible);
    }

    write_input(&mut writer, b"\r");
    wait_for_screen(&output, ROWS, COLS, SUBMISSION_RECEIPT);
    // Successful submission consumes the draft and its history. If undo
    // restores submitted text, the first Ctrl-C clears it instead of exiting.
    write_input(&mut writer, F7);
    write_input(&mut writer, &[3]);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("composer fixture status") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "composer fixture did not exit after undo on the submitted draft:\n{}",
            screen_contents(&output, ROWS, COLS),
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert!(status.success(), "composer fixture exited with {status:?}");
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("PTY reader thread");
}

fn write_input(writer: &mut dyn std::io::Write, bytes: &[u8]) {
    writer.write_all(bytes).expect("PTY input");
    writer.flush().expect("flush PTY input");
}

// Windows ConPTY receives Win32 input records, including key release. Unix
// receives xterm function-key sequences and control bytes in raw mode.
#[cfg(windows)]
const CTRL_Z: &[u8] = b"\x1b[90;44;26;1;8;1_\x1b[90;44;26;0;8;1_";
#[cfg(not(windows))]
const CTRL_Z: &[u8] = b"\x1a";
#[cfg(windows)]
const CTRL_Y: &[u8] = b"\x1b[89;21;25;1;8;1_\x1b[89;21;25;0;8;1_";
#[cfg(not(windows))]
const CTRL_Y: &[u8] = b"\x19";
#[cfg(windows)]
const F7: &[u8] = b"\x1b[118;65;0;1;0;1_\x1b[118;65;0;0;0;1_";
#[cfg(not(windows))]
const F7: &[u8] = b"\x1b[18~";
#[cfg(windows)]
const F8: &[u8] = b"\x1b[119;66;0;1;0;1_\x1b[119;66;0;0;0;1_";
#[cfg(not(windows))]
const F8: &[u8] = b"\x1b[19~";
