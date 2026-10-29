use super::*;
use colossus_contracts::{ProviderUsage, RunPhase};

fn receive(workspace: &mut Workspace, session: &str, run: &str, event: RunEvent) {
    workspace.receive(
        WorkspaceEvent::Session {
            id: session.into(),
            event: Box::new(HostEvent::Run(RunEventEnvelope {
                schema_version: 1,
                run_id: run.into(),
                session_id: session.into(),
                event,
            })),
        },
        host(),
    );
}

fn start(workspace: &mut Workspace, session: &str) {
    workspace.begin_run_usage(true);
    receive(
        workspace,
        session,
        &format!("run-{session}"),
        RunEvent::Phase {
            phase: RunPhase::Preparing,
            turn: None,
            action: None,
            elapsed_seconds: 0.0,
        },
    );
}

fn usage(total: u64) -> RunEvent {
    RunEvent::Provider {
        event: ProviderEvent::Usage {
            usage: ProviderUsage {
                input_tokens: total - 1,
                output_tokens: 1,
                total_tokens: total,
                cached_input_tokens: None,
                reasoning_tokens: None,
            },
        },
    }
}

fn tokens(state: &TuiState) -> Option<u64> {
    state
        .footer
        .usage
        .as_ref()
        .and_then(|report| report.usage.as_ref())
        .map(|usage| usage.total_tokens)
}

#[tokio::test]
async fn background_usage_stays_with_its_session_when_focus_changes() {
    let (mut workspace, _) = Workspace::new(TuiState::from_snapshot(source("alpha")));
    start(&mut workspace, "alpha");
    open_snapshot(&mut workspace, "beta");
    start(&mut workspace, "beta");

    receive(&mut workspace, "alpha", "run-alpha", usage(120));
    assert_eq!(workspace.session_id, "beta");
    assert_eq!(tokens(&workspace), None);
    assert_eq!(tokens(&workspace.background["alpha"]), Some(120));
    receive(&mut workspace, "beta", "run-beta", usage(450));
    receive(&mut workspace, "alpha", "child-alpha", usage(9_999));
    assert_eq!(tokens(&workspace), Some(450));
    assert_eq!(tokens(&workspace.background["alpha"]), Some(120));

    workspace.switch("alpha");
    assert_eq!(tokens(&workspace), Some(120));
    assert_eq!(workspace.footer.usage.as_ref().unwrap().session_id, "alpha");
    receive(&mut workspace, "beta", "run-beta", usage(50));
    assert_eq!(tokens(&workspace), Some(120));
    workspace.switch("beta");
    assert_eq!(tokens(&workspace), Some(500));
    assert_eq!(workspace.footer.usage.as_ref().unwrap().session_id, "beta");
}
