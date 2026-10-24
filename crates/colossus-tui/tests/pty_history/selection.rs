use super::*;

#[test]
fn mouse_drag_highlights_and_copies_while_stream_output_keeps_its_place() {
    for monochrome in [false, true] {
        exercise_mouse_selection(monochrome, false);
    }
}

#[test]
fn inline_mouse_selection_defers_viewport_and_history_until_cleared() {
    exercise_mouse_selection(false, true);
}

fn exercise_mouse_selection(monochrome: bool, inline: bool) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("selection PTY");
    let mut command = fixture_command();
    command.args(["--exact", "fixture_process", "--nocapture"]);
    command.env(
        "COLOSSUS_TUI_MODE",
        if inline { "inline" } else { "alternate" },
    );
    if inline {
        command.env("COLOSSUS_TUI_EMPTY", "1");
    }
    if monochrome {
        command.env("NO_COLOR", "1");
    } else {
        command.env_remove("NO_COLOR");
    }
    for name in [
        "COLOSSUS_TUI_PTY_FIXTURE",
        "COLOSSUS_TUI_STREAM_FIXTURE",
        "COLOSSUS_TUI_SELECTION_FIXTURE",
    ] {
        command.env(name, "1");
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
    if inline {
        wait_for_raw(&output, b"\x1b[6n");
        writer
            .write_all(b"\x1b[1;1R")
            .expect("answer cursor-position query");
        writer.flush().expect("flush cursor-position answer");
    } else {
        #[cfg(windows)]
        answer_cursor_position_query(&output, &mut writer);
    }
    let initial_composer = if inline {
        "Execute · Enter sends"
    } else {
        "Message · Enter sends"
    };
    wait_for_screen(&output, 24, 80, initial_composer);
    writer.write_all(b"inspect\r").expect("submit");
    writer.flush().expect("flush");
    wait_for_screen(&output, 24, 80, "commentary-before-tool");
    let (target, copied) = if inline {
        // Select the live composer: completed transcript rows will otherwise
        // resize this viewport and move into native history during the drag.
        writer
            .write_all(b"\x1b[200~retained-selection-draft\x1b[201~")
            .expect("paste draft while streaming");
        writer.flush().expect("flush draft");
        wait_for_screen(&output, 24, 80, "retained-selection-draft");
        ("retained-selection-draft", "retained")
    } else {
        ("commentary-before-tool", "commentary")
    };
    let rows = screen_rows(&output, 24, 80);
    let (row, line) = rows
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains(target))
        .expect("selection row");
    let byte_column = line.find(target).expect("selection column");
    let column = unicode_width::UnicodeWidthStr::width(&line[..byte_column]);
    let copied_width = unicode_width::UnicodeWidthStr::width(copied);
    // SGR mouse coordinates are one-based. Down, drag, then release the left button.
    let input = format!(
        "\x1b[<0;{};{}M\x1b[<32;{};{}M\x1b[<0;{};{}m",
        column + 1,
        row + 1,
        column + copied_width,
        row + 1,
        column + copied_width,
        row + 1
    );
    writer.write_all(input.as_bytes()).expect("mouse drag");
    writer.flush().expect("flush drag");
    wait_for_screen(&output, 24, 80, "Ctrl+C copy");
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(&output.lock().expect("output"));
    assert!(
        parser
            .screen()
            .cell(row as u16, column as u16)
            .expect("selected cell")
            .inverse(),
        "selection at {row},{column}, inline={inline}, monochrome={monochrome}: {}",
        parser.screen().contents()
    );
    thread::sleep(Duration::from_millis(1100));
    let selected = screen_contents(&output, 24, 80);
    assert!(selected.contains(target), "{selected}");
    assert!(selected.contains("Ctrl+C copy"), "{selected}");
    assert!(!selected.contains("stream-final-row-30"), "{selected}");
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(&output.lock().expect("output"));
    assert!(
        parser
            .screen()
            .cell(row as u16, (column + copied_width - 1) as u16)
            .expect("last selected cell")
            .inverse(),
        "drag highlight must redraw and survive stream completion: {selected}"
    );
    if inline {
        assert!(
            !scrollback_contains(&output, 24, 80, "stream-final-row-01"),
            "captured selection must defer native history advancement"
        );
    }
    writer.write_all(&[3]).expect("copy selection");
    writer.flush().expect("flush copy");
    use base64::Engine as _;
    let clipboard = format!(
        "\x1b]52;c;{}\x07",
        base64::engine::general_purpose::STANDARD.encode(copied)
    );
    wait_for_raw(&output, clipboard.as_bytes());
    writer.write_all(b"\x1b").expect("clear selection");
    writer.flush().expect("flush clear");
    wait_for_screen(&output, 24, 80, "stream-final-row-30");
    if inline {
        wait_for_screen(&output, 24, 80, target);
        assert!(
            scrollback_contains(&output, 24, 80, "stream-final-row-01"),
            "clearing selection must release completed output into native history"
        );
    } else {
        writer
            .write_all(b"!printf 'two  spaces'")
            .expect("shell draft");
        writer.flush().expect("flush shell draft");
        wait_for_screen(&output, 24, 80, "Shell · Enter runs");
        writer.write_all(b"\r").expect("submit user shell");
        writer.flush().expect("flush shell submit");
        wait_for_screen(
            &output,
            24,
            80,
            "Shell fixture received: printf 'two  spaces'",
        );
    }
    writer.write_all(&[3, 3]).expect("exit");
    writer.flush().expect("flush exit");
    assert!(child.wait().expect("fixture status").success());
    drop(writer);
    drop(pair.master);
    reader_task.join().expect("reader task");
}
