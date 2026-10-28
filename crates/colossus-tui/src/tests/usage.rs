use super::*;
use colossus_contracts::{ProviderUsage, RunPhase};

fn counters() -> ProviderUsage {
    ProviderUsage {
        input_tokens: 1_000,
        output_tokens: 250,
        total_tokens: 1_250,
        cached_input_tokens: Some(600),
        reasoning_tokens: Some(100),
    }
}

fn report(run_id: &str, usage: Option<ProviderUsage>) -> RunTokenUsage {
    RunTokenUsage {
        run_id: run_id.into(),
        session_id: "019f-test".into(),
        through_version: 8,
        reports: u64::from(usage.is_some()),
        usage,
    }
}

fn phase(run_id: &str) -> RunEventEnvelope {
    RunEventEnvelope {
        schema_version: 1,
        run_id: run_id.into(),
        session_id: "019f-test".into(),
        event: RunEvent::Phase {
            phase: RunPhase::Preparing,
            turn: None,
            action: None,
            elapsed_seconds: 0.0,
        },
    }
}

fn usage_event(run_id: &str, usage: ProviderUsage) -> RunEventEnvelope {
    RunEventEnvelope {
        event: RunEvent::Provider {
            event: ProviderEvent::Usage { usage },
        },
        ..phase(run_id)
    }
}

fn usage_cards(state: &TuiState) -> usize {
    state
        .transcript
        .iter()
        .filter(|entry| {
            entry.document.blocks.iter().any(|block|
        matches!(block, PresentationBlock::Card { title, .. } if title.starts_with("Run usage")))
        })
        .count()
}

#[test]
fn reducer_counts_equal_reports_separately_without_mixing_children_or_sessions() {
    let mut state = RunUsageState::restored("019f-test", None);
    state.begin("019f-test", true);
    assert!(!state.observe(&usage_event("child", counters())));
    assert!(state.current().is_none());
    state.observe(&phase("direct"));
    state.observe(&usage_event("direct", counters()));
    state.observe(&usage_event("direct", counters()));
    state.observe(&phase("child"));
    assert!(!state.observe(&usage_event("child", counters())));
    let mut wrong_session = usage_event("direct", counters());
    wrong_session.session_id = "other".into();
    assert!(!state.observe(&wrong_session));
    let total = state.current().expect("run usage");
    assert_eq!(total.run_id, "direct");
    assert_eq!(total.reports, 2);
    assert_eq!(total.usage.as_ref().expect("reported").total_tokens, 2_500);
    state.finish();
    assert!(!state.observe(&usage_event("direct", counters())));
    assert_eq!(state.current().expect("retained").reports, 2);
}

#[test]
fn canonical_refresh_replaces_live_counts_and_unknown_details_stay_unknown() {
    let mut state = RunUsageState::restored("019f-test", None);
    state.begin("019f-test", true);
    state.observe(&phase("direct"));
    state.observe(&usage_event("direct", counters()));
    state.replace_canonical("019f-test", Some(report("direct", Some(counters()))));
    assert_eq!(state.current().expect("baseline").reports, 1);
    state.observe(&usage_event(
        "direct",
        ProviderUsage {
            cached_input_tokens: None,
            reasoning_tokens: None,
            ..counters()
        },
    ));
    let total = state.current().expect("run usage");
    assert_eq!(total.reports, 2);
    assert_eq!(total.usage.as_ref().expect("reported").total_tokens, 2_500);
    assert_eq!(
        total.usage.as_ref().expect("reported").cached_input_tokens,
        None
    );
    assert_eq!(
        total.usage.as_ref().expect("reported").reasoning_tokens,
        None
    );
    let partial = total.clone();
    state.replace_canonical("019f-test", None);
    assert_eq!(state.current(), Some(&partial));
    state.begin("019f-test", true);
    assert!(state.current().is_none());
    state.observe(&phase("next"));
    assert_eq!(state.current().expect("unreported").usage, None);
    state.observe(&usage_event(
        "next",
        ProviderUsage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cached_input_tokens: Some(0),
            reasoning_tokens: Some(0),
        },
    ));
    assert_eq!(
        state
            .current()
            .expect("reported zero")
            .usage
            .as_ref()
            .expect("zero")
            .total_tokens,
        0
    );
    state.begin("new-session", false);
    assert!(state.current().is_none());
}

#[test]
fn wrong_same_session_canonical_run_cannot_replace_observed_direct_run() {
    let mut state = RunUsageState::restored("019f-test", None);
    state.begin("019f-test", false);
    state.observe(&phase("goal-iteration-one"));
    state.observe(&usage_event("goal-iteration-one", counters()));
    state.observe(&phase("goal-iteration-two"));
    state.observe(&usage_event("goal-iteration-two", counters()));
    assert_eq!(state.current().expect("one direct run").reports, 1);
    state.replace_canonical(
        "019f-test",
        Some(report("goal-iteration-two", Some(counters()))),
    );
    let finished = state.finish().expect("observed direct run");
    assert_eq!(finished.run_id, "goal-iteration-one");
    assert_eq!(finished.usage.expect("reported").total_tokens, 1_250);
    state.replace_canonical("019f-test", Some(report("idle-latest", Some(counters()))));
    assert_eq!(state.current().expect("idle refresh").run_id, "idle-latest");
}

#[test]
fn sequential_direct_runs_reset_after_terminal_without_counting_a_child_run() {
    for terminal_phase in [RunPhase::Completed, RunPhase::Cancelled] {
        let mut state = RunUsageState::restored("019f-test", None);
        state.begin("019f-test", false);
        state.observe(&phase("iteration-one"));
        state.observe(&usage_event("iteration-one", counters()));
        assert!(!state.observe(&phase("child")));
        assert!(!state.observe(&usage_event("child", counters())));
        let mut terminal = phase("iteration-one");
        terminal.event = RunEvent::Phase {
            phase: terminal_phase,
            turn: Some(1),
            action: None,
            elapsed_seconds: 1.0,
        };
        state.observe(&terminal);
        assert!(!state.observe(&usage_event("iteration-one", counters())));
        assert!(state.observe(&phase("iteration-two")));
        let current = state.current().expect("new direct run");
        assert_eq!(current.run_id, "iteration-two");
        assert_eq!(current.reports, 0);
        assert!(current.usage.is_none());
        state.observe(&usage_event("iteration-two", counters()));
        let current = state.current().expect("second direct run");
        assert_eq!(current.reports, 1);
        assert_eq!(
            current.usage.as_ref().expect("reported").total_tokens,
            1_250
        );
    }
}

#[test]
fn failure_retains_usage_with_events_off_and_transcript_changes() {
    for mode in [EventDisplayMode::Off, EventDisplayMode::Compact] {
        let mut source = snapshot();
        source.preferences.events_mode = mode;
        let mut state = TuiState::from_snapshot(source);
        state.operation = Some(OperationKind::Run);
        state.begin_run_usage(true);
        handle_run_event(&mut state, phase("failed-run"));
        handle_run_event(&mut state, usage_event("failed-run", counters()));
        state.transcript.clear();
        state.transcript_sources.clear();
        handle_host_event(
            &mut state,
            HostEvent::OperationFinished(Box::new(Err("provider failed".into()))),
        );
        assert_eq!(
            state
                .footer
                .usage
                .as_ref()
                .expect("partial usage")
                .usage
                .as_ref()
                .expect("reported")
                .total_tokens,
            1_250
        );
        assert_eq!(usage_cards(&state), 1);
        let rendered = transcript_lines(&state, 100)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("Run usage") && rendered.contains("1,250"),
            "{rendered}"
        );
        state.set_preferences(TerminalPreferences::default());
        assert_eq!(usage_cards(&state), 1);
        state.finish_run_usage();
        assert_eq!(usage_cards(&state), 1);
    }
}

#[test]
fn completed_and_cancelled_turns_show_one_canonical_run_summary() {
    for cancelled in [false, true] {
        let mut state = TuiState::from_snapshot(snapshot());
        state.operation = Some(OperationKind::Run);
        state.begin_run_usage(true);
        handle_run_event(&mut state, phase("direct"));
        handle_run_event(&mut state, usage_event("direct", counters()));
        let outcome = if cancelled {
            AgentRunOutcome::Cancelled {
                result: colossus_contracts::AgentRunCancellation {
                    run_id: "direct".into(),
                    session_id: "019f-test".into(),
                    turn: 1,
                    plan: None,
                    event_count: 8,
                    elapsed_seconds: 0.1,
                },
            }
        } else {
            AgentRunOutcome::Completed {
                result: AgentRunResult {
                    run_id: "direct".into(),
                    session_id: Some("019f-test".into()),
                    role: "primary".into(),
                    profile: "test".into(),
                    model_profile: "test".into(),
                    provider_profile: "test".into(),
                    model: "test".into(),
                    plan: None,
                    output: "Answer".into(),
                    event_count: 8,
                    elapsed_seconds: 0.1,
                },
            }
        };
        let footer = FooterState {
            usage: (!cancelled).then(|| report("direct", Some(counters()))),
            ..state.footer.clone()
        };
        handle_host_event(
            &mut state,
            HostEvent::OperationFinished(Box::new(Ok(OperationResult::Run(HostRunResult {
                outcome,
                footer,
                plan_selection: PlanSelectionUpdate::Unchanged,
            })))),
        );
        assert_eq!(usage_cards(&state), 1);
        let total = state.footer.usage.as_ref().expect("canonical usage");
        assert_eq!(total.reports, 1);
        assert_eq!(total.usage.as_ref().expect("reported").total_tokens, 1_250);
        state.begin_run_usage(false);
        let footer = state.footer.clone();
        handle_host_event(
            &mut state,
            HostEvent::OperationFinished(Box::new(Ok(OperationResult::Command(
                HostCommandResult {
                    footer: Some(footer),
                    ..HostCommandResult::document(PresentationDocument::new())
                },
            )))),
        );
        assert_eq!(
            usage_cards(&state),
            1,
            "a read-only command does not repeat the response summary"
        );
    }
}

#[test]
fn restored_usage_does_not_append_a_response_and_session_switch_resets_it() {
    let mut source = snapshot();
    source.footer.usage = Some(report("restored", Some(counters())));
    let mut state = TuiState::from_snapshot(source);
    assert_eq!(usage_cards(&state), 0);
    state.replace_footer(state.footer.clone());
    state.finish_run_usage();
    assert_eq!(usage_cards(&state), 0);
    apply_command_result(
        &mut state,
        HostCommandResult {
            session: Some((
                "another-session".into(),
                SessionMessagePage {
                    messages: Vec::new(),
                    before_sequence: None,
                    has_more: false,
                },
                None,
            )),
            ..HostCommandResult::document(PresentationDocument::new())
        },
    );
    assert!(state.footer.usage.is_none());
    assert!(state.run_usage.current().is_none());
    let wrong_session = report("old", Some(counters()));
    state.replace_footer(FooterState {
        usage: Some(wrong_session),
        ..FooterState::default()
    });
    assert!(state.footer.usage.is_none());
}

#[test]
fn overflowing_live_counts_are_unavailable_instead_of_wrapping_to_small_numbers() {
    let mut state = RunUsageState::restored("019f-test", None);
    state.begin("019f-test", true);
    state.observe(&phase("direct"));
    state.observe(&usage_event(
        "direct",
        ProviderUsage {
            total_tokens: u64::MAX,
            ..counters()
        },
    ));
    state.observe(&usage_event("direct", counters()));
    assert_eq!(state.current().expect("run").reports, 2);
    assert!(
        state
            .current()
            .expect("overflow unavailable")
            .usage
            .is_none()
    );
    state.observe(&usage_event("direct", counters()));
    assert!(
        state
            .current()
            .expect("overflow remains unavailable")
            .usage
            .is_none()
    );
}

#[test]
fn research_and_goal_execution_clear_prior_usage_but_read_commands_preserve_it() {
    for (name, arguments, starts) in [
        ("research", "why?", true),
        ("research", "list", false),
        ("goal", "objective", true),
        ("goal", "resume goal-a", true),
        ("goals", "", false),
        ("status", "", false),
    ] {
        assert_eq!(
            command_starts_run_usage(&RuntimeCommand::Known {
                name: name.into(),
                arguments: arguments.into()
            }),
            starts
        );
    }
    let mut state =
        RunUsageState::restored("019f-test", Some(report("previous", Some(counters()))));
    state.begin("019f-test", true);
    assert!(
        state.current().is_none(),
        "nonstream research must not show a previous provider run"
    );
    assert!(
        state.finish().is_none(),
        "no direct run events means no invented usage report"
    );
}

#[test]
fn token_badge_preserves_counts_status_permissions_and_security_at_supported_widths() {
    for width in [40, 80, 120] {
        let mut source = snapshot();
        source.footer.usage = Some(report("direct", Some(counters())));
        source.footer.route = "a-very-long-provider-model-name@profile via provider".into();
        source.footer.approval_mode = "full-access".into();
        source
            .security_posture
            .findings
            .push(SecurityPostureFinding {
                code: "test".into(),
                severity: SecurityPostureSeverity::Warning,
                summary: "warning".into(),
                remediation: "remedy".into(),
            });
        let mut state = TuiState::from_snapshot(source);
        state.queue.push_back("next".into());
        state.queue_paused = true;
        let mut terminal = Terminal::new(TestBackend::new(width, FOOTER_HEIGHT)).expect("terminal");
        terminal
            .draw(|frame| render_footer(frame, &state, frame.area(), false))
            .expect("footer");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            rendered.contains("queue paused") && rendered.contains("full-access"),
            "{width}: {rendered}"
        );
        assert!(
            rendered.contains('⚠') && rendered.contains("1.2k"),
            "{width}: {rendered}"
        );
        if width == 120 {
            assert!(
                rendered.contains("1k in / 250 out") && rendered.contains("ctx 1/32768"),
                "{rendered}"
            );
        }
    }
    assert_eq!(compact_token_count(u64::MAX), "18.4E");
    assert_eq!(compact_token_count(999), "999");
    assert_eq!(compact_token_count(1_000), "1k");
    let unknown = report("no-report", None);
    assert_eq!(usage_footer_text(&unknown, 4), "tok—");
    assert!(usage_footer_text(&unknown, 3).is_empty());
}

#[test]
fn compact_usage_preserves_selected_model_and_reasoning_in_sidebar_sized_footer() {
    for width in [110, 111, 112] {
        for usage in [None, Some(counters())] {
            let mut source = snapshot();
            source.footer.route = "live-deep@deep via deep · high reasoning".into();
            source.footer.context = Some((92, 24_800));
            source.footer.approval_mode = "deny".into();
            source.footer.usage = Some(report("direct", usage.clone()));
            source
                .security_posture
                .findings
                .push(SecurityPostureFinding {
                    code: "storage.plaintext".into(),
                    severity: SecurityPostureSeverity::Warning,
                    summary: "Plaintext journal".into(),
                    remediation: "Use storage keys".into(),
                });
            let state = TuiState::from_snapshot(source);
            let mut terminal =
                Terminal::new(TestBackend::new(width, FOOTER_HEIGHT)).expect("terminal");
            terminal
                .draw(|frame| render_footer(frame, &state, frame.area(), false))
                .expect("footer");
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            for expected in [
                "ready",
                "deny",
                "live-deep@deep via deep · high reasoning",
                "ctx 92/24800",
                "⚠ Security: 1",
            ] {
                assert!(rendered.contains(expected), "{width}: {rendered}");
            }
            let tokens = if usage.is_some() { "1.2k" } else { "tok—" };
            assert!(rendered.contains(tokens), "{width}: {rendered}");
        }
    }
}
