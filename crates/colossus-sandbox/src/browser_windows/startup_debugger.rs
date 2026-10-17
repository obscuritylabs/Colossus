//! Auxiliary failure-only stack observation; raw debugger output is never retained.
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt as _, process::Command};

const DEBUGGER: &str = r"C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe";

// The caller retains the exact child process throughout this PID-based attach.
// This runs only after startup has already failed, before ordinary Job cleanup.
pub(super) async fn observe(pid: u32) {
    if std::env::var_os("COLOSSUS_BROWSER_STARTUP_DEBUGGER").as_deref()
        != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    if pid == 0 || !Path::new(DEBUGGER).is_file() {
        eprintln!("networkless CEF debugger status=unavailable");
        return;
    }
    let Some(system_root) = std::env::var_os("SystemRoot") else {
        eprintln!("networkless CEF debugger status=environment_unavailable");
        return;
    };
    let mut command = Command::new(DEBUGGER);
    command
        .args([
            "-pv",
            "-p",
            &pid.to_string(),
            "-noshell",
            "-nosqm",
            "-sins",
            "-netsyms:no",
            "-cf",
            "NUL",
            "-y",
            r"C:\Windows\System32",
            "-c",
            "~* knL 20; qd",
        ])
        .env_clear()
        .env("SystemRoot", system_root)
        .current_dir(Path::new(DEBUGGER).parent().unwrap())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .kill_on_drop(true);
    let Ok(mut debugger) = command.spawn() else {
        eprintln!("networkless CEF debugger status=spawn_failed");
        return;
    };
    // Piped outputs are present because this function configured them above.
    let stdout = debugger.stdout.take().unwrap();
    let stderr = debugger.stderr.take().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::try_join!(
            bounded(stdout, 65_536),
            bounded(stderr, 16_384),
            debugger.wait(),
        )
    })
    .await;
    match result {
        Ok(Ok((stdout, stderr, status))) => {
            let mut stack = frames(&stdout);
            stack.extend(frames(&stderr));
            stack.truncate(128);
            eprintln!(
                "networkless CEF debugger status=completed success={} frames={} output_bytes={} error_bytes={} access_denied={}",
                status.success(),
                stack.len(),
                stdout.len(),
                stderr.len(),
                String::from_utf8_lossy(&stdout)
                    .to_ascii_lowercase()
                    .contains("access is denied")
                    || String::from_utf8_lossy(&stdout).contains("0x80070005")
            );
            for (ordinal, frame) in stack.iter().enumerate() {
                eprintln!("networkless CEF debugger frame={ordinal} site={frame}");
            }
        }
        failure => {
            let reason = if failure.is_err() {
                "timeout"
            } else {
                "io_or_limit"
            };
            let _ = debugger.start_kill();
            let stopped = matches!(
                tokio::time::timeout(Duration::from_secs(3), debugger.wait()).await,
                Ok(Ok(_))
            );
            eprintln!("networkless CEF debugger status={reason} stopped={stopped}");
        }
    }
}

async fn bounded(reader: impl tokio::io::AsyncRead + Unpin, cap: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(cap + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > cap {
        return Err(std::io::Error::other("debugger output bound"));
    }
    Ok(bytes)
}

fn frames(bytes: &[u8]) -> Vec<String> {
    const MODULES: &[&str] = &[
        "ntdll",
        "kernelbase",
        "kernel32",
        "ucrtbase",
        "msvcrt",
        "vcruntime140",
        "vcruntime140_1",
        "win32u",
        "user32",
        "advapi32",
        "rpcrt4",
        "combase",
        "sechost",
        "apphelp",
        "chrome_elf",
        "libcef",
        "wintrust",
        "colossus_native_browser_host",
        "colossus-native-browser-host",
    ];
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_ascii_whitespace().collect();
            let (addresses, site) = match fields.as_slice() {
                [frame, stack, ret, site]
                    if frame.len() == 2 && u8::from_str_radix(frame, 16).ok()? < 32 =>
                {
                    ([*stack, *ret], *site)
                }
                [stack, ret, site] => ([*stack, *ret], *site),
                _ => return None,
            };
            if !addresses.iter().all(|address| {
                (8..=17).contains(&address.len())
                    && address.bytes().all(|b| b.is_ascii_hexdigit() || b == b'`')
            }) {
                return None;
            }
            let (module, symbol) = site.split_once('!')?;
            if !MODULES
                .iter()
                .any(|known| module.eq_ignore_ascii_case(known))
                || symbol.is_empty()
                || symbol.len() > 160
                || !symbol
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_?@$:<>+.-".contains(&b))
            {
                return None;
            }
            Some(format!("{}!{symbol}", module.to_ascii_lowercase()))
        })
        .take(128)
        .collect()
}

#[test]
fn debugger_frames_exclude_paths_addresses_arguments_and_unknown_modules() {
    let output = b"00 00000000`12345678 00000000`87654321 ntdll!NtWaitForSingleObject+0x14\n\
        01 00000000`12345678 00000000`87654321 KERNELBASE!WaitForSingleObjectEx+0x93\n\
        CommandLine: private-profile private-bootstrap\n\
        02 00000000`12345678 00000000`87654321 unknown!private\n\
        03 00000000`12345678 00000000`87654321 ntdll!function private-argument\n\
        04 00000000`12345678 00000000`87654321 ntdll!C:\\private\\file\n";
    assert_eq!(
        frames(output),
        [
            "ntdll!NtWaitForSingleObject+0x14",
            "kernelbase!WaitForSingleObjectEx+0x93",
        ]
    );
    let repeated =
        b"00 00000000`12345678 00000000`87654321 ntdll!NtWaitForSingleObject+0x14\n".repeat(200);
    assert_eq!(frames(&repeated).len(), 128);
    assert_eq!(
        frames(b"00000000`12345678 00000000`87654321 ntdll!NtWaitForSingleObject+0x14\n"),
        ["ntdll!NtWaitForSingleObject+0x14"]
    );
}
