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
            &hard_error_command(),
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
                "networkless CEF debugger status=completed success={} frames={} output_bytes={} error_bytes={} access_denied={} command_error={}",
                status.success(),
                stack.len(),
                stdout.len(),
                stderr.len(),
                String::from_utf8_lossy(&stdout)
                    .to_ascii_lowercase()
                    .contains("access is denied")
                    || String::from_utf8_lossy(&stdout).contains("0x80070005"),
                ["syntax error", "operand error", "memory access error"]
                    .iter()
                    .any(|label| String::from_utf8_lossy(&stdout)
                        .to_ascii_lowercase()
                        .contains(label))
            );
            let missing = missing_dlls(&stdout);
            let hard_error = String::from_utf8_lossy(&stdout)
                .lines()
                .any(|line| line.trim() == "COLOSSUS_LOADER_DLL_NOT_FOUND=1");
            eprintln!(
                "networkless CEF debugger dll_not_found={hard_error} named_dependencies={}",
                missing.len()
            );
            for name in missing {
                eprintln!("networkless CEF debugger missing_dependency={name}");
            }
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

// The previous exact child was parked at this exported x64 syscall. Read only
// its four possible hard-error Unicode parameters, after checking the observed
// instruction and STATUS_DLL_NOT_FOUND. No memory/register writes or continuation.
// The output remains private to the existing bounded in-memory pipe; only the
// fixed dependency names below can be emitted by the fixture.
fn hard_error_command() -> String {
    let mut command = String::from(
        r#".expr /s masm; ~0s; .if (@rip == (ntdll!NtRaiseHardError + 0x14)) { .printf "\nCOLOSSUS_LOADER_DLL_NOT_FOUND=%u\n", ((@r10 & 0xffffffff) == 0xc0000135); .if (((@r10 & 0xffffffff) == 0xc0000135) and (@rdx <= 4) and (@r8 <= 0xf) and (@r9 != 0) and $vvalid(@r9, 0x20)) { "#,
    );
    for index in 0..4 {
        let pointer = format!("poi(@r9 + 0x{:x})", index * 8);
        command.push_str(&format!(
            r#".if ((@rdx > {index}) and ((@r8 & {mask}) != 0)) {{ .if ($vvalid({pointer}, 0x10)) {{ .if ((wo({pointer}) > 0) and (wo({pointer}) <= 0x200)) {{ .if ($vvalid(poi({pointer} + 8), wo({pointer}))) {{ .printf "\nCOLOSSUS_LOADER_NAME=%msu\n", {pointer}; }} }} }} }}; "#,
            mask = 1 << index,
        ));
    }
    command.push_str("} }; ~* knL 20; qd");
    command
}

fn missing_dlls(bytes: &[u8]) -> Vec<&'static str> {
    const KNOWN: &[&str] = &[
        "chrome_elf.dll",
        "kernel32.dll",
        "ntdll.dll",
        "version.dll",
        "libcef.dll",
        "colossus-native-browser-host.dll",
        "ucrtbase.dll",
        "vcruntime140.dll",
        "vcruntime140_1.dll",
        "msvcp140.dll",
    ];
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let value = line.strip_prefix("COLOSSUS_LOADER_NAME=")?;
            if value.len() > 768 || value.chars().any(|c| c.is_control() || c == ';') {
                return None;
            }
            let name = value.rsplit(['\\', '/']).next()?;
            KNOWN
                .iter()
                .copied()
                .find(|known| name.eq_ignore_ascii_case(known))
        })
        .take(4)
        .collect()
}

#[test]
fn missing_dll_observation_releases_only_known_dependency_basenames() {
    assert_eq!(
        missing_dlls(b"COLOSSUS_LOADER_NAME=C:\\private\\CHROME_ELF.DLL\nCOLOSSUS_LOADER_NAME=VERSION.dll\nCOLOSSUS_LOADER_NAME=private.dll\nCOLOSSUS_LOADER_NAME=C:\\private\\chrome_elf.dll;secret\nCOLOSSUS_LOADER_NAME=chrome_elf.dll private-argument\n0:000> COLOSSUS_LOADER_NAME=ntdll.dll\n"),
        ["chrome_elf.dll", "version.dll"]
    );
    assert_eq!(
        missing_dlls(&b"COLOSSUS_LOADER_NAME=ntdll.dll\n".repeat(8)).len(),
        4
    );
}
