use super::*;

#[cfg(windows)]
const CONTROL_C_EXIT: u32 = 0xC000_013A;

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
fn private_directory_creation_protects_the_directory_and_children() {
    let parent = tempfile::tempdir().expect("temporary parent");
    let directory = parent.path().join("private");

    create_private_directory(&directory).expect("create private directory");
    let binding = BoundPath::open_directory(&directory).expect("bind private directory");
    binding
        .validate_private_owner_dacl()
        .expect("private directory DACL");

    let child = directory.join("settings.json");
    std::fs::write(&child, b"{}").expect("write inherited private file");
    let child_binding = BoundPath::open_file(&child).expect("bind inherited file");
    child_binding
        .validate_private_owner_dacl()
        .expect("private child DACL");
    assert!(create_private_directory(&directory).is_err());
}

#[cfg(windows)]
#[test]
fn private_file_replacement_is_atomic_and_preserves_private_access() {
    let parent = tempfile::tempdir().expect("temporary parent");
    let directory = parent.path().join("private");
    create_private_directory(&directory).expect("create private directory");
    let destination = directory.join("settings.json");
    let source = directory.join(".settings.next");
    std::fs::write(&destination, b"old").expect("write original");
    std::fs::write(&source, b"new").expect("write replacement");

    replace_private_file(&source, &destination).expect("replace private file");

    assert!(!source.exists());
    assert_eq!(
        std::fs::read(&destination).expect("read replacement"),
        b"new"
    );
    BoundPath::open_file(&destination)
        .expect("bind replacement")
        .validate_private_owner_dacl()
        .expect("replacement remains private");
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
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(10));
        std::process::exit(2);
    });
    loop {
        std::thread::park();
    }
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
    assert_eq!(
        spawned.child.wait().expect("wait for child"),
        CONTROL_C_EXIT,
        "the ConPTY must deliver Ctrl+C as a Windows console interrupt"
    );
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
