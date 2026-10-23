use super::*;

#[test]
fn agent_status_refreshes_while_idle_and_survives_typing_and_resize() {
    for inline in [false, true] {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY");
        let mut command = fixture_command();
        command.args(["--exact", "fixture_process", "--nocapture"]);
        command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
        command.env("COLOSSUS_TUI_AGENTS_FIXTURE", "1");
        if inline {
            command.env("COLOSSUS_TUI_MODE", "inline");
        }
        let mut child = pair.slave.spawn_command(command).expect("spawn fixture");
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
        if inline || cfg!(windows) {
            wait_for_raw(&output, b"\x1b[6n");
            writer
                .write_all(b"\x1b[1;1R")
                .expect("cursor-position answer");
            writer.flush().expect("flush cursor-position answer");
        }
        wait_for_screen(&output, 24, 120, "Agents · 2 active");
        writer.write_all(b"preserved agent draft").expect("draft");
        writer.flush().expect("flush");
        wait_for_screen(&output, 24, 120, "preserved agent draft");
        // No turn is submitted: the host refresh must observe completion independently.
        wait_for_screen(&output, 24, 120, "Agents · 0 active");
        let complete = screen_contents(&output, 24, 120);
        assert!(
            complete.contains("review") && complete.contains("done"),
            "{complete}"
        );
        let narrow_at = output.lock().expect("output").len();
        pair.master
            .resize(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("narrow resize");
        let narrow = agent_screen(&output, &[(narrow_at, 80)], "preserved agent draft");
        assert!(!narrow.contains("Agents ·"), "{narrow}");
        let wide_at = output.lock().expect("output").len();
        pair.master
            .resize(PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("wide resize");
        let wide = agent_screen(
            &output,
            &[(narrow_at, 80), (wide_at, 120)],
            "Agents · 0 active",
        );
        assert!(wide.contains("preserved agent draft"), "{wide}");
        if inline {
            let bytes = output.lock().expect("output").clone();
            let mut parser = agent_parser(&bytes, &[(narrow_at, 80), (wide_at, 120)]);
            parser.screen_mut().set_scrollback(usize::MAX);
            let history_len = parser.screen().scrollback();
            let mut history = Vec::new();
            for offset in (1..=history_len).rev() {
                parser.screen_mut().set_scrollback(offset);
                history.push(parser.screen().rows(0, 120).next().expect("history row"));
            }
            parser.screen_mut().set_scrollback(0);
            let rows = parser.screen().rows(0, 120).collect::<Vec<_>>();
            let live_start = rows
                .iter()
                .position(|row| row.contains("Agents ·"))
                .expect("live rail");
            history.extend_from_slice(&rows[..live_start]);
            assert!(
                history.iter().all(|row| !row.contains("Agents ·")
                    && !row.contains("├─")
                    && !row.contains("└─")),
                "Agent chrome entered native history: {history:?}"
            );
        }
        writer.write_all(&[3, 3]).expect("clear draft and exit");
        writer.flush().expect("flush exit");
        assert!(child.wait().expect("exit").success());
        drop(writer);
        drop(pair.master);
        reader_thread.join().expect("reader thread");
    }
}

fn agent_screen(output: &Arc<Mutex<Vec<u8>>>, resizes: &[(usize, u16)], needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let bytes = output.lock().expect("output").clone();
        let parser = agent_parser(&bytes, resizes);
        let screen = parser.screen().contents();
        let composer_closed = resizes.last().is_none_or(|&(_, width)| width != 80)
            || screen
                .lines()
                .any(|line| line.contains("Message · Enter sends") && line.ends_with('┐'));
        if screen.contains(needle) && composer_closed {
            return screen;
        }
        assert!(Instant::now() < deadline, "Missing {needle}: {screen}");
        thread::sleep(Duration::from_millis(25));
    }
}

fn agent_parser(bytes: &[u8], resizes: &[(usize, u16)]) -> vt100::Parser {
    let mut parser = vt100::Parser::new(24, 120, 512);
    let mut previous = 0;
    for &(offset, width) in resizes {
        parser.process(&bytes[previous..offset]);
        parser.screen_mut().set_size(24, width);
        previous = offset;
    }
    parser.process(&bytes[previous..]);
    parser
}
