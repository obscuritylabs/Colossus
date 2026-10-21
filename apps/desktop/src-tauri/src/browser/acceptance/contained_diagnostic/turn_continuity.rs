//! Two real Core runs, one suspended page and independently re-admitted Desktop pixels.
use super::{
    BrowserRect, Colossus, ExpectedFrame, Fixture, FixtureState, PAGE_TIMEOUT, SPACE,
    ViewportHeartbeat, accepted_frame, alias, model_tool, remote, request, run_complete,
    tool_result,
};
use crate::state::AppState;
use anyhow::{Context as _, ensure};
use colossus_contracts::BrowserTarget;
use serde_json::{Value, json};
use std::{
    sync::{Mutex, atomic::AtomicBool},
    time::Duration,
};
use tauri::Manager as _;

#[derive(Default)]
pub(super) struct State {
    opened: AtomicBool,
    observed: AtomicBool,
    presented: AtomicBool,
    target: Mutex<Option<(BrowserTarget, u64)>>,
}

pub(super) async fn response(
    phase: usize,
    body: &Value,
    origin: &str,
    state: &FixtureState,
) -> anyhow::Result<Value> {
    let original = state.session_reopen.scope()?;
    match phase {
        10 => Ok(model_tool(
            phase,
            &alias(body, "browser.open")?,
            &json!({"mode":"embedded","allowed_origins":[origin],"initial_url":format!("{origin}/reopened")}),
        )),
        11 => {
            let reply = tool_result(body, "fixture-10")?;
            let summary: colossus_contracts::BrowserSessionSummary =
                serde_json::from_value(reply["session"].clone())?;
            ensure!(
                summary.session_id.as_str() == original.session_id
                    && summary.control_generation > original.control_generation,
                "next run did not resume the exact suspended page with a new writer"
            );
            let tab = summary
                .tabs
                .iter()
                .find(|tab| Some(&tab.tab_id) == summary.selected_tab_id.as_ref())
                .context("resumed selected tab absent")?;
            ensure!(
                tab.tab_id == original.target.tab_id
                    && tab.document_id == original.target.document_id,
                "resumption replaced the retained document"
            );
            state.signal(&state.turn_continuity.opened);
            Ok(model_tool(
                phase,
                &alias(body, "browser.snapshot")?,
                &json!({"session_id":summary.session_id,"control_generation":summary.control_generation,"tab_id":tab.tab_id,"document_id":tab.document_id,"max_nodes":128}),
            ))
        }
        12 => {
            let observed: colossus_contracts::BrowserObservation =
                serde_json::from_value(tool_result(body, "fixture-11")?)?;
            ensure!(
                observed.session_id.as_str() == original.session_id && observed.snapshot.is_some(),
                "resumed snapshot missing"
            );
            let opened = tool_result(body, "fixture-10")?;
            Ok(model_tool(
                phase,
                &alias(body, "browser.navigate")?,
                &json!({"session_id":observed.session_id,"control_generation":opened["control_generation"],"tab_id":observed.tab.tab_id,"document_id":observed.tab.document_id,"url":format!("{origin}/agent")}),
            ))
        }
        13 => {
            let observed: colossus_contracts::BrowserObservation =
                serde_json::from_value(tool_result(body, "fixture-12")?)?;
            ensure!(
                observed.session_id.as_str() == original.session_id
                    && observed.tab.document_id != original.target.document_id,
                "next-run navigation did not preserve the session and advance its document"
            );
            *state.turn_continuity.target.lock().unwrap() = Some((
                BrowserTarget {
                    tab_id: observed.tab.tab_id,
                    document_id: observed.tab.document_id,
                },
                original.control_generation,
            ));
            state.signal(&state.turn_continuity.observed);
            state
                .wait(
                    &state.turn_continuity.presented,
                    "next run's authenticated native page frame",
                )
                .await?;
            Ok(
                json!({"id":"fixture-turn-continuity-complete","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Cross-turn browser session continuity acceptance."},"finish_reason":"stop"}]}),
            )
        }
        _ => anyhow::bail!("unexpected cross-turn continuity phase"),
    }
}

// Keep this ordered two-run native conformance scenario in one reviewable sequence.
#[allow(clippy::too_many_lines)]
pub(super) async fn checkpoint(
    app: &tauri::AppHandle,
    client: &Colossus,
    fixture: &Fixture,
    conversation: &str,
    completed_tab: &str,
    rect: BrowserRect,
    heartbeat: &ViewportHeartbeat,
) -> anyhow::Result<String> {
    let run = client
        .create_run(request(conversation.to_owned())?)
        .await?
        .run;
    *fixture.state.run_id.lock().unwrap() = Some(run.run_id.clone());
    fixture
        .state
        .wait(
            &fixture.state.turn_continuity.opened,
            "new run's browser-open permit",
        )
        .await?;
    fixture
        .state
        .wait(
            &fixture.state.turn_continuity.observed,
            "new run's fresh snapshot",
        )
        .await?;
    let state = app.state::<AppState>();
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    let selected = loop {
        state
            .browser
            .watch_agent(app, SPACE, &run.run_id, conversation)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        let candidate = state
            .browser
            .lock()
            .map_err(|error| anyhow::anyhow!(error.message))?
            .tabs
            .iter()
            .find(|tab| {
                tab.agent_run_id.as_deref() == Some(&run.run_id) && tab.dto.control == "agent"
            })
            .map(|tab| tab.dto.id.clone());
        if let Some(candidate) = candidate {
            break candidate;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "next run never attached its retained browser page"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    ensure!(
        selected != completed_tab,
        "new run reused a completed view's native authority"
    );
    ensure!(
        state
            .browser
            .lock()
            .map_err(|error| anyhow::anyhow!(error.message))?
            .tabs
            .len()
            == 1,
        "each turn accumulated an obsolete completed tab"
    );
    heartbeat.resume(&selected, rect).await?;
    let view = remote(app, &selected)?;
    let (target, old_generation) = fixture
        .state
        .turn_continuity
        .target
        .lock()
        .unwrap()
        .clone()
        .context("fresh resumed target missing")?;
    let probe = accepted_frame(
        app,
        &selected,
        &view,
        rect,
        ExpectedFrame {
            title: "Agent controlled page",
            url: &format!("{}/agent", fixture.origin),
            center_bgra: [216, 247, 217, 255],
            core_target: Some(&target),
            previous: None,
            after: None,
        },
    )
    .await?;
    ensure!(
        probe
            .frame
            .is_some_and(|frame| frame.lease.control_generation > old_generation)
            && !view.human_control_available(),
        "new frame retained an old writer or human input authority"
    );
    fixture
        .state
        .signal(&fixture.state.turn_continuity.presented);
    run_complete(client, &run.run_id).await?;
    state
        .browser
        .watch_agent(app, SPACE, &run.run_id, conversation)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    ensure!(
        state
            .browser
            .lock()
            .map_err(|error| anyhow::anyhow!(error.message))?
            .tabs
            .iter()
            .any(|tab| tab.dto.id == selected
                && tab.dto.control == "completed"
                && tab.dto.completed_snapshot),
        "second completion did not retain the resumed page's authenticated last view"
    );
    println!(
        "PASS two Core runs resume the same session/document with new authority, navigate before Desktop attaches, display a fresh native frame and retain both completed views"
    );
    Ok(selected)
}
