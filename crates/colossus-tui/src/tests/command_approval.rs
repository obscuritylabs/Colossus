use super::*;
use colossus_contracts::CommandApprovalContext;
use colossus_presentation::command_approval_document;

#[test]
fn restored_shell_calls_withhold_unprepared_command_and_credentials() {
    for invocation in [
        serde_json::json!({"command": "echo PRIVATE_COMMAND"}),
        serde_json::json!({"argv": ["echo", "PRIVATE_ARGV"]}),
    ] {
        let mut arguments = invocation;
        arguments["environment"] = serde_json::json!({"TOKEN": "PRIVATE_ENV"});
        arguments["stdin"] = serde_json::json!("PRIVATE_STDIN");
        arguments["justification"] = serde_json::json!("PRIVATE_REASON");
        let mut source = snapshot();
        source.preferences.events_mode = EventDisplayMode::Verbose;
        source.transcript.messages = vec![SessionMessage {
            session_id: "019f-test".into(),
            run_id: "run".into(),
            sequence: 1,
            message: ModelMessage {
                role: ModelMessageRole::Assistant,
                content: String::new().into(),
                tool_call_id: None,
                tool_calls: vec![ModelToolCall {
                    call_id: "call".into(),
                    name: "shell.run".into(),
                    arguments,
                }],
            },
            created_at: "2026-09-09T00:00:00Z".into(),
        }];
        source.transcript.messages.push(SessionMessage {
            session_id: "019f-test".into(), run_id: "run".into(), sequence: 2,
            message: ModelMessage {
                role: ModelMessageRole::Tool, tool_call_id: Some("call".into()), tool_calls: vec![],
                content: serde_json::json!({"invocation": {"command": "PRIVATE_COMMAND"},
                    "resolved_argv": ["PRIVATE_ARGV"], "cwd": "PRIVATE_PATH", "stdout": "SAFE_OUTPUT", "stderr": ""}).to_string().into(),
            }, created_at: "2026-09-09T00:00:01Z".into(),
        });
        let messages = source.transcript.messages;
        // The assistant call may be on an earlier history page. Recognizable
        // shell-result metadata must still not leak before that page is loaded.
        for skip in [0, 1] {
            let mut source = snapshot();
            source.preferences.events_mode = EventDisplayMode::Verbose;
            source.transcript.messages = messages[skip..].to_vec();
            let state = TuiState::from_snapshot(source);
            for width in [32, 80] {
                let rendered = transcript_lines(&state, width)
                    .into_iter()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(!rendered.contains("PRIVATE"), "{rendered}");
                assert!(rendered.contains("withheld"), "{rendered}");
                assert!(rendered.contains("SAFE_OUTPUT"), "{rendered}");
            }
        }
    }
}

#[test]
fn command_tail_remains_reachable_beyond_u16_wrapped_lines_and_after_resize() {
    // Eight accepted 64-KiB arguments can expand this far after replacing a
    // one-character known credential. The display remains below its 4-MiB bound.
    let context = CommandApprovalContext {
        justification: "Check the requested build output.".into(),
        executable: "/bin/echo".into(),
        arguments: vec!["[REDACTED]-".repeat(32_768); 8]
            .into_iter()
            .chain(["ENDTAIL".into()])
            .collect(),
        working_directory: "/work".into(),
        redacted: true,
    };
    context.validate().unwrap();
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "large-command".into(),
            kind: InteractivePromptKind::Approval,
            title: "Approval required".into(),
            document: command_approval_document(&context, None, None, true).unwrap(),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    let Some(Overlay::Prompt {
        document_scroll, ..
    }) = state.overlay.as_mut()
    else {
        panic!("expected pending approval");
    };
    *document_scroll = usize::from(u16::MAX);
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    let Some(Overlay::Prompt {
        document_scroll, ..
    }) = state.overlay.as_mut()
    else {
        panic!("expected pending approval");
    };
    assert!(*document_scroll > usize::from(u16::MAX));
    // A requested offset beyond the document clamps to its final viewport.
    *document_scroll = usize::MAX;
    let mut terminal = Terminal::new(TestBackend::new(40, 32)).unwrap();
    for width in [40, 60] {
        terminal.backend_mut().resize(width, 32);
        terminal.resize(Rect::new(0, 0, width, 32)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .unwrap();
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("ENDTAIL"),
            "tail unreachable at width {width}: {rendered}"
        );
        assert!(
            received.try_recv().is_err(),
            "inspection must not submit approval"
        );
    }
}
