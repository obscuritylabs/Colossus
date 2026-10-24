use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_api::{
    ApiErrorReason, HumanTerminalFrameKind, HumanTerminalOperation as Op, HumanTerminalRequest,
};

fn request(operation: Op) -> HumanTerminalRequest {
    HumanTerminalRequest {
        owner: "human-a".into(),
        operation,
    }
}

#[test]
fn output_ring_reports_gaps_and_preserves_ordered_initial_dimensions() {
    let mut output = output::Output::new(24, 80);
    output.bytes(b"first");
    output.push(HumanTerminalFrameKind::Resize { rows: 30, cols: 90 });
    let (frames, next, gap) = output.read(0, 65536);
    assert!(!gap);
    assert_eq!(next, 3);
    assert_eq!(frames.len(), 3);
    assert!(matches!(
        frames[0].kind,
        HumanTerminalFrameKind::Resize { rows: 24, cols: 80 }
    ));
    for _ in 0..300 {
        output.bytes(&[b'x'; 8192]);
    }
    assert!(output.first() > 1);
    let (frames, _, gap) = output.read(0, 65536);
    assert!(gap);
    assert!(
        frames.is_empty(),
        "a fresh emulator must never receive a suffix as complete history"
    );
    let (frames, _, gap) = output.read(output.first() - 1, 65536);
    assert!(!gap);
    assert!(!frames.is_empty());
}

#[test]
fn osc_filter_handles_split_controls_without_corrupting_unicode() {
    let mut filter = output::OscFilter::default();
    assert_eq!(filter.filter(b"ok\x1b]52;c;secret"), b"ok");
    assert!(filter.filter(b"\x1b").is_empty());
    assert_eq!(filter.filter(b"\\safe\x1b[31mred"), b"safe\x1b[31mred");
    let source = "ĝ🧑中".as_bytes();
    let mut filter = output::OscFilter::default();
    let actual = source
        .iter()
        .flat_map(|byte| filter.filter(&[*byte]))
        .collect::<Vec<_>>();
    assert_eq!(actual, source);
    assert!(filter.filter(b"\xc2").is_empty());
    assert!(filter.filter(b"\x9d52;c;secret\x07").is_empty());
    assert_eq!(filter.filter(b"\x1b\x1b]52;c;secret\x07safe"), b"safe");
    assert_eq!(filter.filter(b"\xc2\x1b]52;c;secret\x07safe"), b"\xc2safe");
}

#[test]
fn terminal_envelopes_reject_unbounded_and_unknown_fields() {
    assert!(
        !request(Op::Read {
            terminal_id: "one".into(),
            after_sequence: 0,
            max_bytes: 65536,
            wait_ms: 1001
        })
        .validate()
    );
    assert!(
        !request(Op::Open {
            request_id: "a".into(),
            controller_id: "b".into(),
            rows: 1,
            cols: 80
        })
        .validate()
    );
    assert!(serde_json::from_str::<HumanTerminalRequest>(r#"{"owner":"human","operation":{"operation":"open","request_id":"a","controller_id":"b","rows":24,"cols":80,"executable":"sh"}}"#).is_err());
}

#[tokio::test]
async fn native_terminal_preserves_process_across_attach_and_deduplicates_input() {
    let workspace = tempfile::tempdir().unwrap();
    let service = HumanTerminalService::new(workspace.path()).unwrap();
    let opened = service
        .execute(
            "app:a",
            request(Op::Open {
                request_id: "open-a".into(),
                controller_id: "page-a".into(),
                rows: 24,
                cols: 80,
            }),
        )
        .await
        .unwrap();
    let session = opened.session.unwrap();
    let id = session.terminal_id.clone();
    let leaked = service
        .execute(
            "app:b",
            request(Op::Read {
                terminal_id: id.clone(),
                after_sequence: 0,
                max_bytes: 65536,
                wait_ms: 0,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(leaked.reason, ApiErrorReason::RunNotFound);
    let mut wrong_owner = request(Op::Attach {
        terminal_id: id.clone(),
        controller_id: "page-b".into(),
        takeover: true,
    });
    wrong_owner.owner = "human-b".into();
    assert!(service.execute("app:a", wrong_owner).await.is_err());
    #[cfg(windows)]
    let command = "[IO.File]::AppendAllText('once.txt','x')\r";
    #[cfg(not(windows))]
    let command = "printf x >> once.txt\r";
    let input = Op::Write {
        terminal_id: id.clone(),
        writer_epoch: 1,
        input_sequence: 1,
        data_base64: STANDARD.encode(command),
    };
    service
        .execute("app:a", request(input.clone()))
        .await
        .unwrap();
    service.execute("app:a", request(input)).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !workspace.path().join("once.txt").exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(
        std::fs::read(workspace.path().join("once.txt")).unwrap(),
        b"x"
    );
    let conflict = service
        .execute(
            "app:a",
            request(Op::Attach {
                terminal_id: id.clone(),
                controller_id: "page-b".into(),
                takeover: false,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(conflict.reason, ApiErrorReason::ConcurrentModification);
    let reattached = service
        .execute(
            "app:a",
            request(Op::Attach {
                terminal_id: id.clone(),
                controller_id: "page-b".into(),
                takeover: true,
            }),
        )
        .await
        .unwrap();
    assert_eq!(reattached.session.unwrap().writer_epoch, 2);
    assert!(
        service
            .execute(
                "app:a",
                request(Op::Write {
                    terminal_id: id.clone(),
                    writer_epoch: 1,
                    input_sequence: 2,
                    data_base64: STANDARD.encode("danger\r")
                })
            )
            .await
            .is_err()
    );
    let replay = service
        .execute(
            "app:a",
            request(Op::Read {
                terminal_id: id.clone(),
                after_sequence: 0,
                max_bytes: 65536,
                wait_ms: 10,
            }),
        )
        .await
        .unwrap();
    assert!(!replay.gap);
    assert!(replay.session.unwrap().complete_history);
    service
        .execute(
            "app:a",
            request(Op::Close {
                terminal_id: id,
                writer_epoch: 2,
            }),
        )
        .await
        .unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn foreground_interrupt_and_close_reach_distinct_shell_job_groups() {
    use colossus_api::HumanTerminalSignal;
    use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
    struct Cleanup(Vec<rustix::fd::OwnedFd>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for child in &self.0 {
                let _ = pidfd_send_signal(child, Signal::KILL);
            }
        }
    }
    fn exited(pid: i32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .is_none_or(|stat| {
                stat.rsplit_once(')')
                    .is_some_and(|(_, tail)| tail.trim_start().starts_with('Z'))
            })
    }
    let workspace = tempfile::tempdir().unwrap();
    let service = HumanTerminalService::new(workspace.path()).unwrap();
    let opened = service
        .execute(
            "app:a",
            request(Op::Open {
                request_id: "job-groups".into(),
                controller_id: "page".into(),
                rows: 24,
                cols: 80,
            }),
        )
        .await
        .unwrap();
    let id = opened.session.unwrap().terminal_id;
    let command = "sleep 60 & printf '%s' \"$!\" > background.pid; (printf '%s' \"$BASHPID\" > foreground.pid; exec sleep 60)\r";
    service
        .execute(
            "app:a",
            request(Op::Write {
                terminal_id: id.clone(),
                writer_epoch: 1,
                input_sequence: 1,
                data_base64: STANDARD.encode(command),
            }),
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !(workspace.path().join("background.pid").exists()
        && workspace.path().join("foreground.pid").exists())
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let foreground = std::fs::read_to_string(workspace.path().join("foreground.pid"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    let background = std::fs::read_to_string(workspace.path().join("background.pid"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    let _cleanup = Cleanup(
        [foreground, background]
            .into_iter()
            .map(|raw| pidfd_open(Pid::from_raw(raw).unwrap(), PidfdFlags::empty()).unwrap())
            .collect(),
    );
    assert_ne!(
        nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(foreground))).unwrap(),
        nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(background))).unwrap(),
        "fixture must exercise separate shell job groups"
    );
    service
        .execute(
            "app:a",
            request(Op::Signal {
                terminal_id: id.clone(),
                writer_epoch: 1,
                signal: HumanTerminalSignal::Interrupt,
            }),
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !exited(foreground) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        exited(foreground),
        "Interrupt must target the foreground job rather than its waiting shell"
    );
    assert!(
        !exited(background),
        "Interrupt must leave the independent background job alone"
    );
    service
        .execute(
            "app:a",
            request(Op::Close {
                terminal_id: id,
                writer_epoch: 1,
            }),
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !exited(background) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        exited(background),
        "Close must clean ordinary background jobs outside the shell's original group"
    );
}

#[tokio::test]
async fn eight_quiet_reads_preserve_input_interrupt_and_close_admission() {
    use colossus_api::HumanTerminalSignal;
    let workspace = tempfile::tempdir().unwrap();
    let service = HumanTerminalService::new(workspace.path()).unwrap();
    let opened = service
        .execute(
            "app:a",
            request(Op::Open {
                request_id: "admission".into(),
                controller_id: "page".into(),
                rows: 24,
                cols: 80,
            }),
        )
        .await
        .unwrap();
    let id = opened.session.unwrap().terminal_id;
    let mut cursor = 0;
    for operation in [
        Op::Write {
            terminal_id: id.clone(),
            writer_epoch: 1,
            input_sequence: 1,
            data_base64: STANDARD.encode("\r"),
        },
        Op::Signal {
            terminal_id: id.clone(),
            writer_epoch: 1,
            signal: HumanTerminalSignal::Interrupt,
        },
        Op::Close {
            terminal_id: id.clone(),
            writer_epoch: 1,
        },
    ] {
        // Prompt output can arrive in separate batches during platform startup.
        // Retry setup if output races the quiet interval; the control assertion
        // below always runs while all eight real long polls remain pending.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let readers = loop {
            loop {
                let read = service
                    .execute(
                        "app:a",
                        request(Op::Read {
                            terminal_id: id.clone(),
                            after_sequence: cursor,
                            max_bytes: 65536,
                            wait_ms: 200,
                        }),
                    )
                    .await
                    .unwrap();
                cursor = read.next_sequence;
                if read.frames.is_empty() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "native fixture never became quiet"
                );
            }
            let mut readers = Vec::new();
            for _ in 0..8 {
                let service = service.clone();
                let id = id.clone();
                readers.push(tokio::spawn(async move {
                    service
                        .execute(
                            "app:a",
                            request(Op::Read {
                                terminal_id: id,
                                after_sequence: cursor,
                                max_bytes: 65536,
                                wait_ms: 1000,
                            }),
                        )
                        .await
                }));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if readers.iter().all(|reader| !reader.is_finished()) {
                break readers;
            }
            for reader in readers {
                reader.abort();
                let _ = reader.await;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "all eight reads must hold admission while quiet"
            );
        };
        service
            .execute("app:a", request(operation))
            .await
            .expect("quiet reads must leave native input and stop controls available");
        for reader in readers {
            reader.await.unwrap().unwrap();
        }
    }
}

#[tokio::test]
async fn natural_shell_exit_cleans_jobs_even_when_a_job_retains_the_pty() {
    let workspace = tempfile::tempdir().unwrap();
    let service = HumanTerminalService::new(workspace.path()).unwrap();
    let opened = service
        .execute(
            "app:a",
            request(Op::Open {
                request_id: "natural-exit".into(),
                controller_id: "page".into(),
                rows: 24,
                cols: 80,
            }),
        )
        .await
        .unwrap();
    let id = opened.session.unwrap().terminal_id;
    #[cfg(windows)]
    let command = "$s=[Diagnostics.ProcessStartInfo]::new([Diagnostics.Process]::GetCurrentProcess().MainModule.FileName);$s.UseShellExecute=$false;$s.CreateNoWindow=$true;$s.Arguments='-NoLogo -NoProfile -NonInteractive -Command Start-Sleep -Seconds 60';$p=[Diagnostics.Process]::Start($s);[IO.File]::WriteAllText('background.pid',[string]$p.Id);exit 0\r";
    #[cfg(target_os = "macos")]
    let command = "unsetopt CHECK_JOBS; sleep 60 & printf '%s' \"$!\" > background.pid; exit 0\r";
    #[cfg(not(any(windows, target_os = "macos")))]
    let command = "sleep 60 & printf '%s' \"$!\" > background.pid; exit 0\r";
    service
        .execute(
            "app:a",
            request(Op::Write {
                terminal_id: id.clone(),
                writer_epoch: 1,
                input_sequence: 1,
                data_base64: STANDARD.encode(command),
            }),
        )
        .await
        .unwrap();
    let mut cursor = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let read = service
            .execute(
                "app:a",
                request(Op::Read {
                    terminal_id: id.clone(),
                    after_sequence: cursor,
                    max_bytes: 65536,
                    wait_ms: 250,
                }),
            )
            .await
            .unwrap();
        cursor = read.next_sequence;
        let session = read.session.unwrap();
        if session.status != colossus_api::HumanTerminalStatus::Running {
            assert_eq!(session.status, colossus_api::HumanTerminalStatus::Exited);
            assert_eq!(
                session.exit_code,
                Some(0),
                "natural completion must preserve the shell exit status"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "live browser reads must not keep a terminal Running after its shell exited"
        );
    }
    assert!(
        workspace.path().join("background.pid").exists(),
        "fixture must launch an ordinary background job before shell exit"
    );
    #[cfg(target_os = "linux")]
    {
        let background = std::fs::read_to_string(workspace.path().join("background.pid")).unwrap();
        let stat = std::fs::read_to_string(format!("/proc/{background}/stat"));
        assert!(
            stat.ok().is_none_or(|stat| stat
                .rsplit_once(')')
                .is_some_and(|(_, tail)| tail.trim_start().starts_with('Z'))),
            "ordinary background job must be ended along with its exited shell"
        );
    }
}
