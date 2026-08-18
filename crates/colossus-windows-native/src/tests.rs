use super::*;

#[cfg(windows)]
const CONTROL_C_EXIT: u32 = 0xC000_013A;

#[cfg(windows)]
const BOOTSTRAP_PIPE_FIXTURE_ENVIRONMENT: &str =
    "COLOSSUS_WINDOWS_NATIVE_BOOTSTRAP_PIPE_FIXTURE_V1";

#[cfg(windows)]
const BOOTSTRAP_PARENT_FIXTURE_ENVIRONMENT: &str =
    "COLOSSUS_WINDOWS_NATIVE_BOOTSTRAP_PARENT_FIXTURE_V1";

#[cfg(windows)]
fn local_app_data_tempdir(prefix: &str) -> tempfile::TempDir {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .expect("absolute Windows LocalAppData");
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(local_app_data)
        .expect("temporary directory under LocalAppData")
}

#[cfg(windows)]
#[test]
fn named_pipe_bootstrap_fixture_process() {
    use std::io::Read as _;

    let Some(pipe_name) = std::env::var_os(BOOTSTRAP_PIPE_FIXTURE_ENVIRONMENT) else {
        return;
    };
    let parent_process = std::env::var(BOOTSTRAP_PARENT_FIXTURE_ENVIRONMENT)
        .expect("bootstrap parent fixture environment")
        .parse::<u32>()
        .expect("bootstrap parent fixture PID");
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe_name)
        .unwrap_or_else(|error| panic!("open bootstrap pipe: {error:?}"));
    validate_named_pipe_server(&pipe, parent_process).unwrap_or_else(|_| std::process::exit(11));
    install_bootstrap_pipe_as_standard_io(&pipe).unwrap_or_else(|_| std::process::exit(12));

    let mut release = [0_u8; 1];
    pipe.read_exact(&mut release)
        .unwrap_or_else(|_| std::process::exit(13));
    assert_eq!(release, [1]);
}

#[cfg(windows)]
#[test]
fn named_pipe_bootstrap_authenticates_the_exact_processes() {
    use tokio::io::AsyncWriteExt as _;
    use tokio::net::windows::named_pipe::ServerOptions;

    let executable = std::fs::canonicalize(std::env::current_exe().expect("test executable"))
        .expect("canonical test executable");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let pipe_name = format!(
        r"\\.\pipe\colossus-windows-native-test-{}-{nonce:x}",
        std::process::id()
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime");
    let runtime_guard = runtime.enter();
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .access_inbound(true)
        .access_outbound(true)
        .create(&pipe_name)
        .expect("bootstrap named-pipe server");
    drop(runtime_guard);
    let mut child = std::process::Command::new(executable)
        .args([
            "--exact",
            "tests::named_pipe_bootstrap_fixture_process",
            "--nocapture",
        ])
        .env_clear()
        .env(BOOTSTRAP_PIPE_FIXTURE_ENVIRONMENT, &pipe_name)
        .env(
            BOOTSTRAP_PARENT_FIXTURE_ENVIRONMENT,
            std::process::id().to_string(),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .expect("spawn bootstrap fixture");
    let child_process = child.id();

    let connection = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), server.connect()).await
    });
    if connection.is_err() {
        let status = child.wait().expect("wait for failed bootstrap fixture");
        panic!("bootstrap fixture connection timeout; child status: {status}");
    }
    connection
        .expect("bootstrap fixture connection timeout")
        .expect("bootstrap fixture connection");
    runtime.block_on(async {
        validate_named_pipe_client(&server, child_process)
            .expect("authenticate bootstrap fixture process");
        server
            .write_all(&[1])
            .await
            .expect("release bootstrap fixture");
    });

    assert!(child.wait().expect("wait for bootstrap fixture").success());
}

#[cfg(windows)]
#[test]
fn suspended_job_bound_bootstrap_authenticates_the_exact_processes() {
    use tokio::io::AsyncWriteExt as _;
    use tokio::net::windows::named_pipe::ServerOptions;

    let executable = std::fs::canonicalize(std::env::current_exe().expect("test executable"))
        .expect("canonical test executable");
    let executable_binding = BoundPath::open_file(&executable).expect("bind test executable");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let pipe_name = format!(
        r"\\.\pipe\colossus-windows-native-suspended-test-{}-{nonce:x}",
        std::process::id()
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime");

    runtime.block_on(async {
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .access_inbound(true)
            .access_outbound(true)
            .create(&pipe_name)
            .expect("bootstrap named-pipe server");
        let mut command = tokio::process::Command::new(&executable);
        command
            .args([
                "--exact",
                "tests::named_pipe_bootstrap_fixture_process",
                "--nocapture",
            ])
            .env_clear()
            .env(BOOTSTRAP_PIPE_FIXTURE_ENVIRONMENT, &pipe_name)
            .env(
                BOOTSTRAP_PARENT_FIXTURE_ENVIRONMENT,
                std::process::id().to_string(),
            )
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        configure_suspended_process(command.as_std_mut());
        let mut child = command.spawn().expect("spawn suspended bootstrap fixture");
        let (job, child_process) = KillOnCloseJob::assign_tokio_child_verify_and_resume(
            &child,
            executable_binding.identity(),
        )
        .expect("bind, verify, and resume bootstrap fixture");

        tokio::time::timeout(std::time::Duration::from_secs(5), server.connect())
            .await
            .expect("bootstrap fixture connection timeout")
            .expect("bootstrap fixture connection");
        validate_named_pipe_client(&server, child_process)
            .expect("authenticate suspended bootstrap fixture process");
        server
            .write_all(&[1])
            .await
            .expect("release suspended bootstrap fixture");

        assert!(
            child
                .wait()
                .await
                .expect("wait for bootstrap fixture")
                .success()
        );
        drop(job);
    });
}

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
fn retained_read_write_handle_writes_and_reports_hard_links() {
    use std::io::{Seek as _, Write as _};

    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("payload.txt");
    create_private_file(&path, b"old").expect("private file");
    let binding = BoundPath::open_file_read_write(&path).expect("read/write binding");
    assert_eq!(binding.link_count().expect("link count"), 1);
    let mut file = binding.try_clone_file().expect("writable clone");
    file.seek(std::io::SeekFrom::Start(0)).expect("seek");
    file.write_all(b"new")
        .expect("write through retained handle");
    file.sync_all().expect("sync retained handle");
    assert_eq!(std::fs::read(&path).expect("read updated file"), b"new");

    std::fs::hard_link(&path, directory.path().join("alias.txt")).expect("hard link");
    assert_eq!(binding.link_count().expect("updated link count"), 2);
}

#[cfg(windows)]
#[test]
fn directories_directly_beneath_the_volume_root_retain_and_validate_the_root() {
    let system_root =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("Windows SystemRoot"));
    assert!(
        system_root
            .parent()
            .is_some_and(|parent| parent.parent().is_none()),
        "SystemRoot must name a directory directly beneath the volume root"
    );

    let retained = crate::windows::open_bound(&system_root, crate::windows::BoundKind::Directory)
        .expect("retain the directory and its volume root");
    assert_eq!(retained.retained_ancestor_count(), 1);

    let binding =
        BoundPath::open_directory(&system_root).expect("bind directory beneath the volume root");
    binding
        .validate_ancestor_namespace_authority()
        .expect("volume root namespace authority");
    binding.revalidate().expect("same directory identity");
}

#[cfg(windows)]
#[test]
fn private_directory_creation_protects_the_directory_and_children() {
    let parent = local_app_data_tempdir("colossus-native-private-directory-");
    let directory = parent.path().join("private");

    create_private_directory(&directory).expect("create private directory");
    let binding = BoundPath::open_directory(&directory).expect("bind private directory");
    binding
        .validate_ancestor_namespace_authority()
        .expect("standard ancestors retain safe namespace authority");
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
fn untrusted_ancestor_delete_child_authority_is_rejected() {
    let parent = tempfile::tempdir().expect("temporary parent");
    let directory = parent.path().join("private");
    create_private_directory(&directory).expect("create private directory");

    let status = std::process::Command::new("icacls.exe")
        .arg(parent.path())
        .args(["/grant", "*S-1-1-0:(DC)"])
        .status()
        .expect("run Windows ACL editor");
    assert!(status.success(), "grant Everyone delete-child authority");

    let binding = BoundPath::open_directory(&directory).expect("bind private directory");
    assert!(
        binding.validate_ancestor_namespace_authority().is_err(),
        "an untrusted principal with FILE_DELETE_CHILD may replay the home namespace"
    );
}

#[cfg(windows)]
#[test]
fn private_file_creation_is_exclusive_and_owner_private() {
    let parent = local_app_data_tempdir("colossus-native-private-file-");
    let path = parent.path().join("secret");

    create_private_file(&path, b"colossus").expect("create private file");

    assert_eq!(std::fs::read(&path).expect("read secret"), b"colossus");
    BoundPath::open_file(&path)
        .expect("bind private file")
        .validate_private_owner_dacl()
        .expect("private file DACL");
    assert!(create_private_file(&path, b"other").is_err());
    assert_eq!(std::fs::read(&path).expect("read secret"), b"colossus");
}

#[cfg(windows)]
#[test]
fn private_file_replacement_is_atomic_and_preserves_private_access() {
    let parent = local_app_data_tempdir("colossus-native-private-replace-");
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
fn private_file_replacement_rejects_untrusted_ancestor_authority() {
    let ancestor = tempfile::tempdir().expect("temporary ancestor");
    let directory = ancestor.path().join("private");
    create_private_directory(&directory).expect("create private directory");
    let destination = directory.join("settings.json");
    let source = directory.join(".settings.next");
    create_private_file(&destination, b"old").expect("private destination");
    create_private_file(&source, b"new").expect("private source");

    let status = std::process::Command::new("icacls.exe")
        .arg(ancestor.path())
        .args(["/grant", "*S-1-1-0:(DC)"])
        .status()
        .expect("run Windows ACL editor");
    assert!(status.success(), "grant Everyone delete-child authority");

    assert!(
        replace_private_file(&source, &destination).is_err(),
        "a replaceable ancestor must block path-based atomic replacement"
    );
    assert_eq!(
        std::fs::read(&destination).expect("read unchanged destination"),
        b"old"
    );
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
