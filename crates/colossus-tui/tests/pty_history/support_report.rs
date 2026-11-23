use super::*;
use std::path::{Path, PathBuf};

const ENABLE_FIXTURE: &str = "COLOSSUS_TUI_SUPPORT_REPORT_FIXTURE";
const DESTINATION: &str = r"C:\Fixture\Support reports\local.json";

pub(super) async fn save_fixture(
    output: PathBuf,
    replace: bool,
    control: RunControl,
) -> Result<(), String> {
    if std::env::var_os(ENABLE_FIXTURE).is_none() || output.as_path() != Path::new(DESTINATION) {
        return Err("Unexpected support report fixture request".into());
    }
    if !replace {
        return Ok(());
    }
    while !control.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err("Support report save cancelled.".into())
}

#[test]
fn support_report_composer_saves_and_cancels_without_echoing_destination() {
    const ROWS: u16 = 24;
    const COLS: u16 = 100;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("support report PTY");
    let mut command = fixture_command();
    command.args(["--exact", "fixture_process", "--nocapture"]);
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    command.env("COLOSSUS_TUI_MODE", "alternate");
    command.env(ENABLE_FIXTURE, "1");
    let mut child = pair
        .slave
        .spawn_command(command)
        .expect("support report fixture");
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
    let command = format!(r#"/support-report "{DESTINATION}""#);
    for (input, notice) in [
        (format!("{command}\r"), "Support report saved."),
        (
            format!("{command} --replace\r"),
            "Saving support report · Esc cancels",
        ),
    ] {
        writer
            .write_all(input.as_bytes())
            .expect("submit support report");
        writer.flush().expect("flush submission");
        wait_for_screen(&output, ROWS, COLS, notice);
        let screen = screen_contents(&output, ROWS, COLS);
        assert!(screen.contains("durable-row-05"));
        assert!(!screen.contains(DESTINATION));
    }

    writer.write_all(b"\x1b").expect("cancel support report");
    writer.flush().expect("flush cancellation");
    wait_for_screen(&output, ROWS, COLS, "Support report save cancelled.");
    writer.write_all(&[3]).expect("exit fixture");
    writer.flush().expect("flush exit");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("fixture status") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "support report fixture did not exit"
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert!(
        status.success(),
        "support report fixture exited with {status:?}"
    );
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("PTY reader thread");
}
