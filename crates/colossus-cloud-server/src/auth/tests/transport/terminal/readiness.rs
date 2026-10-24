use super::*;
use colossus_sdk::HumanTerminalStatus;

/// Apple zsh enables bracketed paste after ZLE terminal initialization.
/// https://github.com/apple-oss-distributions/zsh/blob/main/zsh/Src/Zle/zle_main.c#L1266-L1292
pub(super) async fn wait_for_line_editor(socket: &mut Socket, terminal: &str) {
    // This cursor is local to readiness: main reconnect/replay assertions still
    // consume the complete stream through their original cursor.
    let mut cursor = 0;
    let mut enabled = None;
    let mut tail = Vec::new();
    let ready = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let reply = ok(
                socket,
                HumanTerminalOperation::Read {
                    terminal_id: terminal.into(),
                    after_sequence: cursor,
                    max_bytes: 65536,
                    wait_ms: 200,
                },
            )
            .await;
            assert!(
                !reply.gap,
                "line-editor readiness requires retained history"
            );
            let session = reply.session.unwrap();
            assert_eq!(
                session.status,
                HumanTerminalStatus::Running,
                "shell exited before line-editor readiness; tail: {:?}",
                String::from_utf8_lossy(&tail)
            );
            for frame in reply.frames {
                assert_eq!(frame.sequence, cursor + 1);
                cursor = frame.sequence;
                if let HumanTerminalFrameKind::Output { data_base64 } = frame.kind {
                    tail.extend(STANDARD.decode(data_base64).unwrap());
                    for token in tail.windows(8) {
                        if token == b"\x1b[?2004h" {
                            enabled = Some(true);
                        } else if token == b"\x1b[?2004l" {
                            enabled = Some(false);
                        }
                    }
                    if tail.len() > 8192 {
                        tail.drain(..tail.len() - 8192);
                    }
                }
            }
            assert_eq!(reply.next_sequence, cursor);
            let partial_toggle = (1..8).any(|length| {
                tail.ends_with(&b"\x1b[?2004h"[..length])
                    || tail.ends_with(&b"\x1b[?2004l"[..length])
            });
            if enabled == Some(true) && !partial_toggle && cursor == session.next_sequence {
                break;
            }
        }
    })
    .await;
    assert!(
        ready.is_ok(),
        "zsh did not enter its line editor; final mode: {enabled:?}; tail: {:?}",
        String::from_utf8_lossy(&tail)
    );
}
