//! Shared native PTY fixture; never linked into production.
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    io::{Read as _, Write as _},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

#[path = "terminal_environment.rs"]
mod environment;

pub(crate) struct Terminal {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>,
    screen: Arc<Mutex<ScreenOutput>>,
    changed: Arc<Condvar>,
}

struct ScreenOutput {
    parser: vt100::Parser,
    observed_at: Instant,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Terminal {
    pub(crate) fn start(command: CommandBuilder) -> Self {
        Self::start_timed(command).0
    }

    pub(crate) fn start_timed(mut command: CommandBuilder) -> (Self, Instant) {
        environment::configure(&mut command);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY");
        let started = Instant::now();
        let child = pair.slave.spawn_command(command).expect("CLI TUI");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let writer = Arc::new(Mutex::new(pair.master.take_writer().expect("writer")));
        let screen = Arc::new(Mutex::new(ScreenOutput {
            parser: vt100::Parser::new(40, 100, 1000),
            observed_at: started,
        }));
        let changed = Arc::new(Condvar::new());
        let output = Arc::clone(&screen);
        let response = Arc::clone(&writer);
        let notify = Arc::clone(&changed);
        thread::spawn(move || {
            let mut bytes = [0; 8192];
            let mut query_tail = Vec::new();
            while let Ok(count) = reader.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                {
                    let mut output = output.lock().expect("screen");
                    output.parser.process(&bytes[..count]);
                    output.observed_at = Instant::now();
                    notify.notify_all();
                }
                query_tail.extend_from_slice(&bytes[..count]);
                if query_tail.windows(4).any(|window| window == b"\x1b[6n") {
                    let mut writer = response.lock().expect("response writer");
                    let _ = writer.write_all(b"\x1b[1;1R");
                    let _ = writer.flush();
                }
                let start = query_tail.len().saturating_sub(3);
                query_tail.drain(..start);
            }
        });
        (
            Self {
                child,
                master: pair.master,
                writer,
                screen,
                changed,
            },
            started,
        )
    }

    pub(crate) fn send(&self, bytes: &[u8]) {
        let mut writer = self.writer.lock().expect("writer");
        writer.write_all(bytes).expect("terminal input");
        writer.flush().expect("flush input");
    }

    pub(crate) fn wait(&self, text: &str) -> String {
        self.wait_until(text, |screen| screen.contains(text))
    }

    pub(crate) fn wait_until(&self, description: &str, ready: impl Fn(&str) -> bool) -> String {
        self.wait_until_observed(description, Duration::from_secs(20), ready)
            .0
    }

    pub(crate) fn wait_until_observed(
        &self,
        description: &str,
        timeout: Duration,
        ready: impl Fn(&str) -> bool,
    ) -> (String, Instant) {
        let deadline = Instant::now() + timeout;
        let mut output = self.screen.lock().expect("screen");
        loop {
            let screen = output.parser.screen().contents();
            if ready(&screen) {
                return (screen, output.observed_at);
            }
            assert!(
                Instant::now() < deadline,
                "TUI never rendered {description:?}:\n{screen}"
            );
            output = self
                .changed
                .wait_timeout(output, deadline.saturating_duration_since(Instant::now()))
                .expect("screen update")
                .0;
        }
    }

    #[cfg(feature = "journal-startup-benchmark")]
    pub(crate) fn wait_for_clean_exit(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("CLI exited with {status}"))
                };
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err("CLI teardown timed out".into())
    }

    pub(crate) fn wait_until_visual_rows(
        &self,
        description: &str,
        ready: impl Fn(&str, &[String]) -> bool,
    ) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let (contents, rows) = {
                let parser = self.screen.lock().expect("screen");
                let screen = parser.parser.screen();
                // ConPTY may soft-wrap adjacent visual rows. contents() joins
                // those rows, so layout assertions need the physical row view.
                let (_, columns) = screen.size();
                (
                    screen.contents(),
                    screen.rows(0, columns).collect::<Vec<_>>(),
                )
            };
            if ready(&contents, &rows) {
                return contents;
            }
            assert!(
                Instant::now() < deadline,
                "TUI never rendered {description:?}:\n{}",
                rows.join("\n")
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub(crate) fn command(&self, command: &str, expected: &str) -> String {
        // An earlier transcript entry may already contain `expected`. Observe
        // this input in the composer before submitting, then wait for both its
        // consumption and the host operation's completed render.
        self.send(format!("\x1b[200~{command}\x1b[201~").as_bytes());
        self.wait_until_visual_rows(&format!("composer accepted {command:?}"), |_, rows| {
            composer_contents(rows).is_some_and(|draft| draft.starts_with(command))
        });
        self.send(b"\r");
        self.wait_until_visual_rows(
            &format!("{expected:?} after {command:?}"),
            |screen, rows| {
                screen.contains(expected)
                    && !screen.contains("running /")
                    && composer_contents(rows).is_some_and(|draft| draft.is_empty())
            },
        )
    }

    pub(crate) fn wait_for_composer(
        &self,
        description: &str,
        ready: impl Fn(&str) -> bool,
    ) -> String {
        self.wait_until_visual_rows(description, |_, rows| {
            composer_contents(rows).is_some_and(|draft| ready(&draft))
        })
    }

    pub(crate) fn resize(&self, rows: u16, cols: u16) {
        self.resize_frame(rows, cols);
        // A truncated old frame can still contain every plugin name. Require
        // the closing border at the new chat width before sending more input.
        // A wide viewport reserves 28..=36 columns for the adaptive status rail.
        self.wait_until_visual_rows("resized composer border", |_, rows| {
            rows.iter().any(|line| {
                (line.starts_with("┌ Message") || line.starts_with("┌ Execute"))
                    && ((line.ends_with('┐') && line.chars().count() == usize::from(cols))
                        || (cols >= 92
                            && (28..=36).any(|rail| {
                                let boundary = usize::from(cols - rail);
                                line.chars().nth(boundary - 1) == Some('┐')
                                    && line.chars().nth(boundary) == Some('│')
                            })))
            })
        });
    }

    pub(crate) fn resize_frame(&self, rows: u16, cols: u16) {
        // Resize the emulator before the child can publish a new-width frame.
        let mut screen = self.screen.lock().expect("screen");
        screen.parser.screen_mut().set_size(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize");
        drop(screen);
    }
}

fn composer_contents(rows: &[String]) -> Option<String> {
    let mut lines = rows
        .iter()
        .skip_while(|line| !line.starts_with("┌ Message") && !line.starts_with("┌ Execute"));
    lines.next()?;
    Some(
        lines
            .take_while(|line| !line.starts_with('└'))
            // The adjacent composer and status-rail borders bound the draft.
            .map(|line| {
                line.split_once("││")
                    .map_or(line.as_str(), |(chat, _)| chat)
                    .trim_matches('│')
                    .trim()
            })
            .collect(),
    )
}
