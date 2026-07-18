use super::{SpawnRequest, WindowsProcessError, spawn};
use std::{collections::BTreeMap, path::PathBuf};

#[test]
fn rejects_relative_and_unbounded_launch_contracts_before_platform_dispatch() {
    let mut request = SpawnRequest {
        executable: PathBuf::from("relative.exe"),
        arguments: Vec::new(),
        cwd: PathBuf::from("."),
        environment: BTreeMap::new(),
        appcontainer_sid: "S-1-15-2-1".into(),
        max_processes: 1,
        max_memory_bytes: 1024,
        proxy_port: None,
        network_filter_id: None,
    };
    assert!(matches!(
        spawn(&request),
        Err(WindowsProcessError::Invalid(_))
    ));
    request.executable = PathBuf::from("/absolute.exe");
    request.cwd = PathBuf::from("/");
    request.proxy_port = Some(42);
    assert!(matches!(
        spawn(&request),
        Err(WindowsProcessError::Invalid(_))
    ));
}

#[cfg(windows)]
#[test]
fn create_process_current_directory_uses_drive_syntax_after_canonicalization() {
    let encoded =
        super::windows_impl::wide_process_path(std::path::Path::new(r"\\?\C:\workspace\allowed"));
    assert_eq!(
        String::from_utf16(&encoded[..encoded.len() - 1]).expect("UTF-16 path"),
        r"C:\workspace\allowed"
    );
}

#[cfg(windows)]
#[test]
fn create_process_environment_uses_drive_syntax_after_canonicalization() {
    let block = super::windows_impl::environment_block(&BTreeMap::from([
        ("TARGET".into(), r"\\?\C:\workspace\allowed.txt".into()),
        ("UNC".into(), r"\\?\UNC\server\share".into()),
    ]));
    let decoded = String::from_utf16(&block).expect("UTF-16 environment");
    assert!(decoded.contains("TARGET=C:\\workspace\\allowed.txt\0"));
    assert!(decoded.contains("UNC=\\\\?\\UNC\\server\\share\0"));
}

#[cfg(windows)]
#[test]
fn cmd_command_payload_uses_cmd_quote_semantics_instead_of_crt_escaping() {
    let command = super::windows_impl::windows_command_line(
        std::ffi::OsStr::new(r"\\?\C:\Windows\System32\cmd.exe"),
        &[
            "/D".into(),
            "/S".into(),
            "/C".into(),
            "type \"%TARGET%\"".into(),
        ],
    );
    assert_eq!(
        command,
        "\\\\?\\C:\\Windows\\System32\\cmd.exe /D /S /C \"type \"%TARGET%\"\""
    );
}
