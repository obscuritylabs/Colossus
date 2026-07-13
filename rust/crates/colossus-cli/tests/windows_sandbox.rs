//! Native Windows AppContainer, Job Object, filesystem, environment, and network acceptance.
#![cfg(windows)]

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::Duration,
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const SIGNING_KEY: &str = "8888888888888888888888888888888888888888888888888888888888888888";

fn run<I, S>(binary: &Path, config: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(binary)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_WINDOWS_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_WINDOWS_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

fn yaml_path(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).expect("YAML path")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "result JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn decoded(value: &Value, field: &str) -> Vec<u8> {
    BASE64
        .decode(value[field].as_str().expect("base64 field"))
        .expect("base64")
}

struct WindowsTools {
    cmd: PathBuf,
    curl: PathBuf,
    powershell: PathBuf,
}

impl WindowsTools {
    fn discover() -> Self {
        let windows = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        Self {
            cmd: windows.join("System32").join("cmd.exe"),
            curl: windows.join("System32").join("curl.exe"),
            powershell: windows
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_config(
    config: &Path,
    root: &Path,
    allowed: &Path,
    workflows: &Path,
    tools: &WindowsTools,
    max_processes: u32,
    max_memory_bytes: u64,
    timeout_ms: u64,
    network_destinations: &[String],
) {
    let network_destinations = serde_json::to_string(network_destinations).expect("destinations");
    fs::write(
        config,
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_WINDOWS_TEST_JOURNAL_KEY
    journal_key_id: windows-test-journal-v1
    signing_variable: COLOSSUS_WINDOWS_TEST_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: [process.spawn]
  approval_actions: []
  require_post_effect: false
workflows:
  repository: {workflows}
  user: {workflows}
sandbox:
  backend: windows_job
  profile: windows-appcontainer-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {allowed}
      mode: write
  executables: [{cmd}, {curl}, {powershell}]
  environment: [SAFE, TARGET]
  networkDestinations: {network_destinations}
  timeoutMs: {timeout_ms}
  maxOutputBytes: 1048576
  maxProcesses: {max_processes}
  maxMemoryBytes: {max_memory_bytes}
  maxConcurrency: 1
"#,
            state = yaml_path(&root.join("state.redb")),
            anchor = yaml_path(&root.join("anchor.json")),
            workflows = yaml_path(workflows),
            allowed = yaml_path(allowed),
            cmd = yaml_path(&tools.cmd),
            curl = yaml_path(&tools.curl),
            powershell = yaml_path(&tools.powershell),
        ),
    )
    .expect("config");
}

fn process<'a>(
    executable: &'a Path,
    cwd: &'a Path,
    environment: &[(&'a str, &'a str)],
    arguments: &'a [&'a str],
) -> Vec<String> {
    let mut command = vec![
        "process".into(),
        "run".into(),
        executable.to_string_lossy().into_owned(),
        "--cwd".into(),
        cwd.to_string_lossy().into_owned(),
    ];
    for (name, value) in environment {
        command.extend(["--env".into(), format!("{name}={value}")]);
    }
    if !arguments.is_empty() {
        command.push("--".into());
        command.extend(arguments.iter().map(|argument| (*argument).into()));
    }
    command
}

#[test]
fn windows_appcontainer_enforces_filesystem_environment_job_and_network_boundaries() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
    let directory = tempdir().expect("directory");
    let root = fs::canonicalize(directory.path()).expect("root");
    let allowed = root.join("allowed");
    let denied = root.join("denied");
    let workflows = root.join("workflows");
    fs::create_dir_all(&allowed).expect("allowed");
    fs::create_dir_all(&denied).expect("denied");
    fs::create_dir_all(&workflows).expect("workflows");
    let allowed_file = allowed.join("allowed.txt");
    let denied_file = denied.join("denied.txt");
    fs::write(&allowed_file, b"allowed-content\r\n").expect("allowed file");
    fs::write(&denied_file, b"denied-secret\r\n").expect("denied file");
    let tools = WindowsTools::discover();
    assert!(tools.cmd.is_file(), "cmd.exe is required");
    assert!(tools.curl.is_file(), "curl.exe is required");
    assert!(tools.powershell.is_file(), "Windows PowerShell is required");
    let config = root.join("config.yaml");
    write_config(
        &config,
        &root,
        &allowed,
        &workflows,
        &tools,
        2,
        268_435_456,
        5_000,
        &[],
    );

    let doctor = run(binary, &config, &["sandbox", "doctor"]);
    assert!(
        doctor.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor = json(&doctor);
    assert_eq!(doctor["platform"], "windows");
    assert_eq!(doctor["native_supported"], true);
    assert!(
        doctor["native_details"].as_str().is_some_and(
            |details| details.contains("AppContainer") && details.contains("Job Object")
        )
    );

    let target = allowed_file.to_string_lossy();
    let allowed_read = run(
        binary,
        &config,
        &process(
            &tools.cmd,
            &allowed,
            &[("TARGET", target.as_ref())],
            &["/D", "/S", "/C", "type \"%TARGET%\""],
        ),
    );
    assert!(
        allowed_read.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed_read.stderr)
    );
    let allowed_read = json(&allowed_read);
    assert_eq!(allowed_read["backend"], "windows_job");
    assert_eq!(allowed_read["success"], true, "{allowed_read}");
    assert!(
        decoded(&allowed_read, "stdout_base64")
            .windows(b"allowed-content".len())
            .any(|window| window == b"allowed-content")
    );

    let denied_target = denied_file.to_string_lossy();
    let denied_read = run(
        binary,
        &config,
        &process(
            &tools.cmd,
            &allowed,
            &[("TARGET", denied_target.as_ref())],
            &["/D", "/S", "/C", "type \"%TARGET%\""],
        ),
    );
    assert!(denied_read.status.success());
    let denied_read = json(&denied_read);
    assert_eq!(denied_read["success"], false);
    assert!(
        !decoded(&denied_read, "stdout_base64")
            .windows(b"denied-secret".len())
            .any(|window| window == b"denied-secret")
    );

    let traversal = allowed.join("..").join("denied").join("denied.txt");
    let traversal = traversal.to_string_lossy();
    let traversal_read = run(
        binary,
        &config,
        &process(
            &tools.cmd,
            &allowed,
            &[("TARGET", traversal.as_ref())],
            &["/D", "/S", "/C", "type \"%TARGET%\""],
        ),
    );
    assert!(traversal_read.status.success());
    assert_eq!(json(&traversal_read)["success"], false);

    let denied_marker = denied.join("escaped.txt");
    let denied_marker_target = denied_marker.to_string_lossy();
    let denied_write = run(
        binary,
        &config,
        &process(
            &tools.cmd,
            &allowed,
            &[("TARGET", denied_marker_target.as_ref())],
            &["/D", "/S", "/C", "echo escaped> \"%TARGET%\""],
        ),
    );
    assert!(denied_write.status.success());
    assert_eq!(json(&denied_write)["success"], false);
    assert!(!denied_marker.exists());

    let environment = run(
        binary,
        &config,
        &process(
            &tools.cmd,
            &allowed,
            &[("SAFE", "visible")],
            &["/D", "/S", "/C", "set"],
        ),
    );
    assert!(environment.status.success());
    let environment = decoded(&json(&environment), "stdout_base64");
    let environment = String::from_utf8_lossy(&environment);
    assert!(environment.contains("SAFE=visible"));
    assert!(!environment.contains("COLOSSUS_SANDBOX_JOB_KEY"));
    assert!(!environment.contains(JOURNAL_KEY));
    assert!(!environment.contains(SIGNING_KEY));

    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let origin = format!("http://{address}");
    let url = format!("{origin}/allowed");
    let raw_network = run(
        binary,
        &config,
        &process(
            &tools.curl,
            &allowed,
            &[],
            &["--silent", "--show-error", "--max-time", "1", &url],
        ),
    );
    assert!(raw_network.status.success());
    assert_eq!(json(&raw_network)["success"], false);
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));

    write_config(
        &config,
        &root,
        &allowed,
        &workflows,
        &tools,
        2,
        268_435_456,
        5_000,
        std::slice::from_ref(&origin),
    );
    listener.set_nonblocking(false).expect("blocking listener");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("proxy upstream connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).expect("read proxy request");
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
        assert!(request.starts_with("get /allowed http/1.1\r\n"));
        assert!(!request.contains("proxy-authorization:"));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nallowed-network",
            )
            .expect("write proxy response");
    });
    let allowed_network = run(
        binary,
        &config,
        &process(
            &tools.curl,
            &allowed,
            &[],
            &[
                "--silent",
                "--show-error",
                "--fail-with-body",
                "--max-time",
                "5",
                &url,
            ],
        ),
    );
    assert!(
        allowed_network.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed_network.stderr)
    );
    let allowed_network = json(&allowed_network);
    assert_eq!(allowed_network["success"], true);
    assert_eq!(
        decoded(&allowed_network, "stdout_base64"),
        b"allowed-network"
    );
    server.join().expect("proxy upstream server");

    let environment = run(
        binary,
        &config,
        &process(
            &tools.cmd,
            &allowed,
            &[],
            &["/D", "/S", "/C", "set HTTP_PROXY"],
        ),
    );
    assert!(environment.status.success());
    let environment = decoded(&json(&environment), "stdout_base64");
    let environment = String::from_utf8_lossy(&environment);
    assert!(environment.contains("http://colossus:[REDACTED]@127.0.0.1:"));

    let bypass = TcpListener::bind("127.0.0.1:0").expect("bypass listener");
    bypass.set_nonblocking(true).expect("bypass nonblocking");
    let bypass_origin = format!("http://{}", bypass.local_addr().expect("bypass address"));
    write_config(
        &config,
        &root,
        &allowed,
        &workflows,
        &tools,
        2,
        268_435_456,
        5_000,
        std::slice::from_ref(&bypass_origin),
    );
    let direct_bypass = run(
        binary,
        &config,
        &process(
            &tools.curl,
            &allowed,
            &[],
            &[
                "--silent",
                "--show-error",
                "--noproxy",
                "*",
                "--max-time",
                "1",
                &bypass_origin,
            ],
        ),
    );
    assert!(direct_bypass.status.success());
    assert_eq!(json(&direct_bypass)["success"], false);
    assert!(matches!(
        bypass.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));

    let wrong_auth = run(
        binary,
        &config,
        &process(
            &tools.curl,
            &allowed,
            &[],
            &[
                "--silent",
                "--show-error",
                "--fail-with-body",
                "--proxy-header",
                "Proxy-Authorization: Basic Y29sb3NzdXM6d3Jvbmc=",
                "--max-time",
                "2",
                &bypass_origin,
            ],
        ),
    );
    assert!(wrong_auth.status.success());
    assert_eq!(json(&wrong_auth)["success"], false);
    assert!(matches!(
        bypass.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));

    let unlisted = TcpListener::bind("127.0.0.1:0").expect("unlisted listener");
    unlisted
        .set_nonblocking(true)
        .expect("unlisted nonblocking");
    let unlisted_url = format!(
        "http://{}",
        unlisted.local_addr().expect("unlisted address")
    );
    let unlisted_request = run(
        binary,
        &config,
        &process(
            &tools.curl,
            &allowed,
            &[],
            &[
                "--silent",
                "--show-error",
                "--fail-with-body",
                "--max-time",
                "2",
                &unlisted_url,
            ],
        ),
    );
    assert!(unlisted_request.status.success());
    assert_eq!(json(&unlisted_request)["success"], false);
    assert!(matches!(
        unlisted.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));

    write_config(
        &config,
        &root,
        &allowed,
        &workflows,
        &tools,
        1,
        268_435_456,
        5_000,
        &[],
    );
    let process_limit = run(
        binary,
        &config,
        &process(
            &tools.cmd,
            &allowed,
            &[],
            &[
                "/D",
                "/S",
                "/C",
                "start \"\" /B cmd.exe /D /S /C exit 0 & choice /D Y /T 2 >NUL",
            ],
        ),
    );
    assert!(!process_limit.status.success());
    assert!(String::from_utf8_lossy(&process_limit.stderr).contains("process-count"));

    write_config(
        &config,
        &root,
        &allowed,
        &workflows,
        &tools,
        2,
        268_435_456,
        5_000,
        &[],
    );
    let memory_limit = run(
        binary,
        &config,
        &process(
            &tools.powershell,
            &allowed,
            &[],
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$memory = [byte[]]::new(536870912); for ($offset = 0; $offset -lt $memory.Length; $offset += 4096) { $memory[$offset] = 1 }; Start-Sleep -Seconds 2",
            ],
        ),
    );
    assert!(
        !memory_limit.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&memory_limit.stdout),
        String::from_utf8_lossy(&memory_limit.stderr)
    );
    assert!(
        String::from_utf8_lossy(&memory_limit.stderr).contains("memory"),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&memory_limit.stdout),
        String::from_utf8_lossy(&memory_limit.stderr)
    );

    write_config(
        &config,
        &root,
        &allowed,
        &workflows,
        &tools,
        2,
        268_435_456,
        500,
        &[],
    );
    let child_marker = allowed.join("child-escaped.txt");
    let child_marker_target = child_marker.to_string_lossy();
    let timed_out = run(
        binary,
        &config,
        &process(
            &tools.cmd,
            &allowed,
            &[("TARGET", child_marker_target.as_ref())],
            &[
                "/D",
                "/S",
                "/C",
                "start \"\" /B cmd.exe /D /S /C \"choice /D Y /T 2 >NUL & echo escaped> \\\"%TARGET%\\\"\" & choice /D Y /T 30 >NUL",
            ],
        ),
    );
    assert!(!timed_out.status.success());
    assert!(String::from_utf8_lossy(&timed_out.stderr).contains("exceeded its timeout"));
    thread::sleep(Duration::from_secs(3));
    assert!(
        !child_marker.exists(),
        "a descendant escaped Job Object cleanup"
    );
}
