//! Exact disposable launchd jobs used only by native session/port acceptance.

use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

pub struct Fixture {
    directory: tempfile::TempDir,
    executable: PathBuf,
}

impl Fixture {
    pub fn compile() -> Self {
        let directory = tempfile::tempdir().expect("private native session fixture directory");
        let executable = directory.path().join("session-ports-probe");
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session_ports_probe.c");
        let output = Command::new("/usr/bin/xcrun")
            .args([
                "clang",
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-Wno-deprecated-declarations",
            ])
            .arg(source)
            .args(["-lbsm", "-o"])
            .arg(&executable)
            .output()
            .expect("native SDK compiler required");
        assert!(
            output.status.success(),
            "native fixture compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            directory,
            executable,
        }
    }

    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
}

pub struct Job<'a> {
    _fixture: &'a Fixture,
    label: String,
    target: String,
    stdout: PathBuf,
    stderr: PathBuf,
    removed: bool,
}

impl<'a> Job<'a> {
    pub fn start(
        fixture: &'a Fixture,
        role: &str,
        args: &[&str],
        fresh: bool,
        suspended: bool,
    ) -> Self {
        let nonce = fixture.directory().file_name().unwrap().to_str().unwrap();
        let label = format!(
            "com.colossus.development.native-session-{}-{role}-{nonce}",
            std::process::id()
        );
        let parent =
            colossus_darwin_process::DarwinProcessIdentity::bind(std::process::id()).unwrap();
        let domain = format!("gui/{}", parent.real_uid());
        let target = format!("{domain}/{label}");
        let stdout = fixture.directory().join(format!("{role}.stdout"));
        let stderr = fixture.directory().join(format!("{role}.stderr"));
        for path in [&stdout, &stderr] {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
        }
        let service = if role == "receiver" { &label } else { "" };
        let mut arguments = vec![fixture.executable.to_str().unwrap(), role];
        if role == "receiver" {
            arguments.push(service);
        }
        arguments.extend_from_slice(args);
        let program = arguments
            .iter()
            .map(|value| format!("<string>{}</string>", xml(value)))
            .collect::<String>();
        let mach_service = if role == "receiver" {
            format!(
                "<key>MachServices</key><dict><key>{}</key><true/></dict>",
                xml(service)
            )
        } else {
            String::new()
        };
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array>{program}</array><key>RunAtLoad</key><true/><key>KeepAlive</key><false/><key>SessionCreate</key><{}/><key>WaitForDebugger</key><{}/><key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string>{mach_service}</dict></plist>",
            xml(&label),
            if fresh { "true" } else { "false" },
            if suspended { "true" } else { "false" },
            xml(stdout.to_str().unwrap()),
            xml(stderr.to_str().unwrap())
        );
        let path = fixture.directory().join(format!("{role}.plist"));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap()
            .write_all(plist.as_bytes())
            .unwrap();
        let job = Self {
            _fixture: fixture,
            label,
            target,
            stdout,
            stderr,
            removed: false,
        };
        let bootstrapped = Command::new("/bin/launchctl")
            .args(["bootstrap", &domain])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            bootstrapped.status.success(),
            "exact owned job bootstrap failed: {}",
            String::from_utf8_lossy(&bootstrapped.stderr)
        );
        job
    }

    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn stdout(&self) -> &Path {
        &self.stdout
    }
    pub fn stderr(&self) -> &Path {
        &self.stderr
    }

    fn inspect(&self) -> Output {
        Command::new("/bin/launchctl")
            .args(["print", &self.target])
            .output()
            .unwrap()
    }

    pub fn wait_for_pid(&self) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = self.inspect();
            assert!(
                output.status.success(),
                "owned job must remain queryable before resume"
            );
            if let Some(pid) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("pid = ")
                        .and_then(|pid| pid.parse().ok())
                })
            {
                return pid;
            }
            assert!(
                Instant::now() < deadline,
                "owned launchd process allocation deadline"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn wait_for_output(&self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if fs::read_to_string(&self.stdout).unwrap().contains(expected) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "owned fixture output deadline; stdout={} stderr={} job={}",
                fs::read_to_string(&self.stdout).unwrap(),
                fs::read_to_string(&self.stderr).unwrap(),
                String::from_utf8_lossy(&self.inspect().stdout)
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn wait_for_exit_zero(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = self.inspect();
            assert!(
                output.status.success(),
                "exact owned job remains installed until removal"
            );
            let text = String::from_utf8_lossy(&output.stdout);
            if text.lines().any(|line| line.trim() == "last exit code = 0")
                && !text.lines().any(|line| line.trim().starts_with("pid = "))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "owned job did not acknowledge zero exit: {text}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn remove_and_require_absent(&mut self) {
        let removed = Command::new("/bin/launchctl")
            .args(["bootout", &self.target])
            .output()
            .unwrap();
        assert!(removed.status.success(), "exact owned job removal failed");
        let query = self.inspect();
        assert_eq!(
            query.status.code(),
            Some(113),
            "positive exact-job absence status"
        );
        assert!(
            String::from_utf8_lossy(&query.stderr)
                .contains(&format!("Could not find service \"{}\"", self.label))
        );
        self.removed = true;
    }
}

impl Drop for Job<'_> {
    fn drop(&mut self) {
        if !self.removed {
            let _ = Command::new("/bin/launchctl")
                .args(["bootout", &self.target])
                .output();
        }
    }
}

pub fn wait_stopped(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = Command::new("/bin/ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        if output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .trim_start()
                .starts_with('T')
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "exact launchd process suspension deadline"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
