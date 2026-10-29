use super::*;

#[test]
fn work_and_mcp_status_show_without_agents_and_success_changes_the_bubble() {
    for inline in [false, true] {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 32,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("status PTY");
        let mut command = fixture_command();
        command.args(["--exact", "fixture_process", "--nocapture"]);
        for flag in [
            "COLOSSUS_TUI_PTY_FIXTURE",
            "COLOSSUS_TUI_STREAM_FIXTURE",
            "COLOSSUS_TUI_STATUS_FIXTURE",
        ] {
            command.env(flag, "1");
        }
        command.env_remove("NO_COLOR");
        if inline {
            command.env("COLOSSUS_TUI_MODE", "inline");
        }
        let mut child = pair.slave.spawn_command(command).expect("fixture");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let output = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&output);
        let reader_task = thread::spawn(move || {
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
        let mut writer = pair.master.take_writer().expect("writer");
        if inline || cfg!(windows) {
            wait_for_raw(&output, b"\x1b[6n");
            writer.write_all(b"\x1b[1;1R").expect("cursor answer");
            writer.flush().expect("flush cursor answer");
        }
        wait_for_screen(&output, 32, 120, "Memory 24 · Plans 2");
        let ready = screen_contents(&output, 32, 120);
        assert!(ready.contains("Tasks 1 · Goals 1"), "{ready}");
        assert!(ready.contains("Review runtime"), "{ready}");
        if !ready.contains("atlas") {
            writer.write_all(b"\x1b[6;3~").expect("scroll status");
            writer.flush().expect("flush status scroll");
        }
        wait_for_screen(&output, 32, 120, "atlas");
        let configured = screen_rows(&output, 32, 120)
            .into_iter()
            .find(|row| row.contains("atlas"))
            .expect("server row");
        assert!(
            configured.contains("○") && configured.contains("configured"),
            "{configured}"
        );
        writer.write_all(b"inspect\r").expect("submit");
        writer.flush().expect("flush submit");
        wait_for_screen(&output, 32, 120, "used");
        wait_for_screen(&output, 32, 120, "Message · Enter sends");
        if !screen_contents(&output, 32, 120).contains("atlas") {
            writer
                .write_all(b"\x1b[6;3~")
                .expect("scroll completed status");
            writer.flush().expect("flush completed status scroll");
        }
        wait_for_screen(&output, 32, 120, "atlas");
        let mut parser = vt100::Parser::new(32, 120, 0);
        parser.process(&output.lock().expect("output"));
        let screen = parser.screen();
        let columns = screen.size().1;
        let row = screen
            .rows(0, columns)
            .position(|row| row.contains("atlas") && row.contains("used"))
            .expect("used server row") as u16;
        let sidebar_heading_row = screen
            .rows(0, columns)
            .position(|row| row.contains("Workspace sessions"))
            .expect("workspace sidebar heading") as u16;
        let sidebar_border = (0..columns)
            .rev()
            .find(|column| {
                screen
                    .cell(sidebar_heading_row, *column)
                    .is_some_and(|cell| cell.contents() == "│")
            })
            .expect("rendered sidebar left border");
        let bubble = (sidebar_border + 1..columns)
            .find_map(|column| {
                screen
                    .cell(row, column)
                    .filter(|cell| cell.contents() == "●")
            })
            .expect("bubble");
        let palette = colossus_presentation::TerminalPalette::for_preferences(
            &TerminalPreferences::default(),
        );
        let green = palette
            .tone_style(PresentationTone::Success)
            .foreground
            .expect("success color");
        assert_eq!(
            bubble.fgcolor(),
            vt100::Color::Rgb(green.red, green.green, green.blue)
        );
        if !screen_contents(&output, 32, 120).contains("Message · Enter sends") {
            wait_for_screen(&output, 32, 120, "Message · Enter sends");
        }
        writer.write_all(&[3, 3]).expect("exit");
        writer.flush().expect("flush exit");
        assert!(child.wait().expect("fixture status").success());
        drop(writer);
        drop(pair.master);
        reader_task.join().expect("reader task");
    }
}
