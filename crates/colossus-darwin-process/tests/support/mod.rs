//! Bounded owned native fixtures; no browser or whole-tree acceptance claim.
use colossus_darwin_process::{
    DarwinChild, DarwinProcessIdentity, DarwinProcessSignal, spawn_suspended_pipes,
};
use std::{
    ffi::OsString,
    fs::File,
    io::{BufRead as _, BufReader, Write as _},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Program {
    directory: tempfile::TempDir,
    executable: PathBuf,
}

impl Program {
    pub fn compile() -> Self {
        let directory = tempfile::tempdir().expect("private native fixture directory");
        let executable = directory.path().join("process-probe");
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let result = Command::new("/usr/bin/xcrun")
            .args([
                "clang",
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-Wno-deprecated-declarations",
            ])
            .arg(fixtures.join("process_probe.c"))
            .arg(fixtures.join("syscall_probe.c"))
            .args(["-lsandbox", "-o"])
            .arg(&executable)
            .output()
            .expect("native SDK compiler required");
        assert!(
            result.status.success(),
            "fixed native fixture compilation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Self {
            directory,
            executable,
        }
    }

    pub fn spawn(&self, scenario: &str) -> Probe<'_> {
        let spawned = spawn_suspended_pipes(&self.executable, &[OsString::from(scenario)], &[])
            .expect("exact suspended native fixture");
        let (sender, receiver) = mpsc::sync_channel(128);
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(spawned.output);
            loop {
                let mut line = String::new();
                match output.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(bytes) if bytes <= 1024 => {
                        if sender.send(line.trim_end().to_owned()).is_err() {
                            break;
                        }
                    }
                    Ok(_) => break,
                }
            }
        });
        let mut probe = Probe {
            _program: self,
            child: spawned.child,
            input: Some(spawned.input),
            receiver,
            reader: Some(reader),
            descendants: Vec::new(),
        };
        probe.child.resume().expect("resume verified fixture");
        probe
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }
}

pub struct Probe<'a> {
    _program: &'a Program,
    pub child: DarwinChild,
    input: Option<File>,
    pub receiver: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    descendants: Vec<DarwinProcessIdentity>,
}

impl Probe<'_> {
    pub fn line(&self) -> String {
        self.receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("bounded owned fixture receipt")
    }

    pub fn command(&mut self, command: u8) {
        self.input
            .as_mut()
            .expect("owned input")
            .write_all(&[command])
            .expect("fixed native fixture command");
    }

    pub fn retain_descendant(&mut self, identity: DarwinProcessIdentity) {
        self.descendants.push(identity);
    }

    pub fn wait(&mut self) -> ExitStatus {
        self.input.take();
        let status = self.child.wait().expect("positively reap direct fixture");
        self.reader
            .take()
            .expect("owned output reader")
            .join()
            .expect("join output reader after physical exit");
        status
    }
}

impl Drop for Probe<'_> {
    fn drop(&mut self) {
        self.input.take();
        for identity in &self.descendants {
            let _ = identity.signal(DarwinProcessSignal::Kill);
        }
        let _ = self.child.kill_and_reap();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub fn number(line: &str, key: &str) -> i64 {
    line.split_whitespace()
        .find_map(|word| word.split_once('=').filter(|(name, _)| *name == key))
        .expect("fixed receipt field")
        .1
        .parse()
        .expect("bounded numeric receipt")
}

pub fn wait_state(pid: u32, state: char) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = Command::new("/bin/ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .output()
            .expect("read-only exact fixture process state");
        if output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .trim_start()
                .starts_with(state)
        {
            return;
        }
        assert!(Instant::now() < deadline, "fixture process state deadline");
        thread::sleep(Duration::from_millis(10));
    }
}
