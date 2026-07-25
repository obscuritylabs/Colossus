use super::*;

#[cfg(windows)]
#[test]
fn retained_file_handle_can_read_and_revalidate() {
    use std::io::Read as _;

    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("payload.txt");
    std::fs::write(&path, b"colossus").expect("write fixture");

    let binding = BoundPath::open_file(&path).expect("bind file");
    let mut file = binding.try_clone_file().expect("clone retained file");
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .expect("read retained file");

    assert_eq!(contents, "colossus");
    binding.revalidate().expect("same file identity");
}

#[cfg(windows)]
#[test]
fn conpty_fixture_process() {
    use std::io::{Read as _, Write as _};

    if std::env::var_os("COLOSSUS_DESKTOP_TUI_AUTH_INPUT_HANDLE_V1").is_none() {
        return;
    }
    let mut channels = take_desktop_tui_authentication_channels().expect("authentication channels");
    let mut request = [0_u8; 4];
    channels
        .input
        .read_exact(&mut request)
        .expect("authentication request");
    assert_eq!(&request, b"ping");
    channels
        .output
        .write_all(b"pong")
        .expect("authentication response");
    let mut interrupt = [0_u8; 1];
    std::io::stdin()
        .read_exact(&mut interrupt)
        .expect("ConPTY interrupt");
    assert_eq!(interrupt, [0x03]);
}

#[cfg(windows)]
#[test]
fn conpty_authentication_resize_interrupt_and_job_cleanup() {
    use std::io::{Read as _, Write as _};

    let executable = std::fs::canonicalize(std::env::current_exe().expect("test executable"))
        .expect("canonical test executable");
    let executable_binding = BoundPath::open_file(&executable).expect("bind executable");
    let workspace = tempfile::tempdir().expect("workspace");
    let workspace = std::fs::canonicalize(workspace.path()).expect("canonical test workspace");
    let workspace_binding = BoundPath::open_directory(&workspace).expect("bind workspace");
    let environment = ["SystemRoot", "WINDIR", "TEMP", "TMP", "PATH"]
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (name.into(), value)))
        .collect::<Vec<_>>();
    let mut spawned = spawn_verified_conpty(
        &executable,
        executable_binding.identity(),
        &[
            "--exact".into(),
            "tests::conpty_fixture_process".into(),
            "--nocapture".into(),
        ],
        &environment,
        &workspace,
        workspace_binding.identity(),
        24,
        80,
    )
    .expect("verified ConPTY");
    spawned
        .authentication_input
        .write_all(b"ping")
        .expect("write authentication");
    let mut response = [0_u8; 4];
    spawned
        .authentication_output
        .read_exact(&mut response)
        .expect("read authentication");
    assert_eq!(&response, b"pong");
    spawned.control.resize(40, 120).expect("resize ConPTY");
    spawned.control.interrupt().expect("interrupt ConPTY");
    assert_eq!(spawned.child.wait().expect("wait for child"), 0);
    spawned
        .control
        .terminate()
        .expect("idempotent process cleanup");
}

#[cfg(not(windows))]
#[test]
fn non_windows_calls_fail_closed() {
    assert!(matches!(
        BoundPath::open_directory(std::path::Path::new("/tmp")),
        Err(WindowsNativeError::UnsupportedPlatform)
    ));
    assert!(matches!(
        prompt_secret("Title", "Message", "target", 32),
        Err(WindowsNativeError::UnsupportedPlatform)
    ));
}
