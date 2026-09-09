//! Shared native PTY fixture; never linked into production.
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    io::{Read as _, Write as _},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub(crate) struct Terminal {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>,
    screen: Arc<Mutex<vt100::Parser>>,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Terminal {
    pub(crate) fn start(command: CommandBuilder) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("PTY");
        let child = pair.slave.spawn_command(command).expect("CLI TUI");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let writer = Arc::new(Mutex::new(pair.master.take_writer().expect("writer")));
        let screen = Arc::new(Mutex::new(vt100::Parser::new(40, 100, 1000)));
        let output = Arc::clone(&screen);
        let response = Arc::clone(&writer);
        thread::spawn(move || {
            let mut bytes = [0; 8192];
            let mut query_tail = Vec::new();
            while let Ok(count) = reader.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                output.lock().expect("screen").process(&bytes[..count]);
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
        Self {
            child,
            master: pair.master,
            writer,
            screen,
        }
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
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let screen = self.screen.lock().expect("screen").screen().contents();
            if ready(&screen) {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "TUI never rendered {description:?}:\n{screen}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub(crate) fn command(&self, command: &str, expected: &str) -> String {
        // An earlier transcript entry may already contain `expected`. Observe
        // this input in the composer before submitting, then wait for both its
        // consumption and the host operation's completed render.
        self.send(format!("\x1b[200~{command}\x1b[201~").as_bytes());
        self.wait_until(&format!("composer accepted {command:?}"), |screen| {
            composer_contents(screen).is_some_and(|draft| draft.starts_with(command))
        });
        self.send(b"\r");
        self.wait_until(&format!("{expected:?} after {command:?}"), |screen| {
            screen.contains(expected)
                && !screen.contains("running /")
                && composer_contents(screen).is_some_and(|draft| draft.is_empty())
        })
    }

    pub(crate) fn resize(&self, rows: u16, cols: u16) {
        self.resize_frame(rows, cols);
        // A truncated old frame can still contain every plugin name. Require
        // the closing border at the new width before sending any more input.
        self.wait_until("resized composer border", |screen| {
            screen.lines().any(|line| {
                line.starts_with("┌ Message")
                    && line.ends_with('┐')
                    && line.chars().count() == usize::from(cols)
            })
        });
    }

    pub(crate) fn resize_frame(&self, rows: u16, cols: u16) {
        // Resize the emulator before the child can publish a new-width frame.
        let mut screen = self.screen.lock().expect("screen");
        screen.screen_mut().set_size(rows, cols);
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

fn composer_contents(screen: &str) -> Option<String> {
    let mut lines = screen
        .lines()
        .skip_while(|line| !line.starts_with("┌ Message") && !line.starts_with("┌ Execute"));
    lines.next()?;
    Some(
        lines
            .take_while(|line| !line.starts_with('└'))
            .map(|line| line.trim_matches('│').trim())
            .collect(),
    )
}
