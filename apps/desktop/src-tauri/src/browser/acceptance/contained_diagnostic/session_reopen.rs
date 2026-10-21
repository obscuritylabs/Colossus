//! Real same-run session replacement after acknowledged Core close, before completion.
use super::{
    BrowserRect, ExpectedFrame, Fixture, FixtureState, PAGE_TIMEOUT, SPACE, ViewportHeartbeat,
    accepted_frame, alias, model_tool, remote, tool_result, windows,
};
use crate::state::AppState;
use anyhow::{Context as _, ensure};
use colossus_contracts::{
    BrowserControlState, BrowserMode, BrowserObservation, BrowserSessionSummary, BrowserTarget,
};
use serde_json::{Value, json};
use std::{
    sync::{Mutex, atomic::AtomicBool},
    time::Duration,
};
use tauri::Manager as _;

pub(super) const TITLE: &str = "Reopened agent page";
pub(super) const PAGE: &[u8] = br#"<!doctype html><html><head><title>Reopened agent page</title></head><body style="background:#ffe8cc"><h1>Reopened agent page</h1><p>Fresh session B after exact session A close.</p></body></html>"#;

#[derive(Clone)]
pub(super) struct Reopened {
    pub(super) session_id: String,
    pub(super) target: BrowserTarget,
    pub(super) control_generation: u64,
}

#[derive(Default)]
pub(super) struct State {
    closed: AtomicBool,
    gap_observed: AtomicBool,
    observed: AtomicBool,
    presented: AtomicBool,
    scope: Mutex<Option<Reopened>>,
}

impl State {
    pub(super) fn scope(&self) -> anyhow::Result<Reopened> {
        self.scope
            .lock()
            .unwrap()
            .clone()
            .context("reopened session is absent")
    }

    pub(super) fn session_id(&self) -> anyhow::Result<String> {
        Ok(self.scope()?.session_id)
    }
}

pub(super) struct RunIdentity<'a> {
    pub run_id: &'a str,
    pub conversation_id: &'a str,
    pub first_tab: &'a str,
    pub first_session: &'a str,
}

pub(super) async fn response(
    phase: usize,
    body: &Value,
    origin: &str,
    state: &FixtureState,
) -> anyhow::Result<Value> {
    let first = state
        .target
        .lock()
        .unwrap()
        .clone()
        .context("first session is absent")?;
    match phase {
        6 => Ok(model_tool(
            phase,
            &alias(body, "browser.close")?,
            &json!({"session_id":first["session_id"],"control_generation":first["control_generation"]}),
        )),
        7 => {
            let closed = tool_result(body, "fixture-6")?;
            ensure!(
                closed["closed"] == true && closed["session_id"] == first["session_id"],
                "Core did not positively close the exact first session"
            );
            state.signal(&state.session_reopen.closed);
            state
                .wait(
                    &state.session_reopen.gap_observed,
                    "Desktop context polls before the replacement session opens",
                )
                .await?;
            Ok(model_tool(
                phase,
                &alias(body, "browser.open")?,
                &json!({"mode":"embedded","allowed_origins":[origin],"initial_url":format!("{origin}/reopened")}),
            ))
        }
        8 => {
            let reply = tool_result(body, "fixture-7")?;
            let opened: BrowserSessionSummary = serde_json::from_value(reply["session"].clone())?;
            ensure!(
                Some(opened.session_id.as_str()) != first["session_id"].as_str()
                    && opened.mode == BrowserMode::Embedded
                    && opened.control == BrowserControlState::Agent
                    && opened.control_generation > 0
                    && reply["control_generation"].as_u64() == Some(opened.control_generation),
                "Core did not open a distinct embedded agent session"
            );
            let tab = opened
                .tabs
                .iter()
                .find(|tab| Some(&tab.tab_id) == opened.selected_tab_id.as_ref())
                .context("reopened selected tab is absent")?;
            *state.session_reopen.scope.lock().unwrap() = Some(Reopened {
                session_id: opened.session_id.to_string(),
                control_generation: opened.control_generation,
                target: BrowserTarget {
                    tab_id: tab.tab_id.clone(),
                    document_id: tab.document_id.clone(),
                },
            });
            Ok(model_tool(
                phase,
                &alias(body, "browser.snapshot")?,
                &json!({"session_id":opened.session_id,"control_generation":opened.control_generation,"tab_id":tab.tab_id,"document_id":tab.document_id,"max_nodes":128}),
            ))
        }
        9 => {
            let observed: BrowserObservation =
                serde_json::from_value(tool_result(body, "fixture-8")?)?;
            let scope = state.session_reopen.scope()?;
            ensure!(
                observed.session_id.as_str() == scope.session_id
                    && observed.tab.tab_id == scope.target.tab_id
                    && (observed.tab.title.is_empty() || observed.tab.title == TITLE)
                    && observed.snapshot.is_some(),
                "fresh snapshot did not describe the exact reopened page"
            );
            *state.session_reopen.scope.lock().unwrap() = Some(Reopened {
                session_id: scope.session_id,
                control_generation: scope.control_generation,
                target: BrowserTarget {
                    tab_id: observed.tab.tab_id,
                    document_id: observed.tab.document_id,
                },
            });
            state.signal(&state.session_reopen.observed);
            state
                .wait(
                    &state.session_reopen.presented,
                    "reopened session's native frame",
                )
                .await?;
            Ok(
                json!({"id":"fixture-natural-complete","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Same-run close, reopen, and natural completion acceptance."},"finish_reason":"stop"}]}),
            )
        }
        _ => anyhow::bail!("unreviewed close/reopen fixture phase"),
    }
}

pub(super) async fn checkpoint(
    app: &tauri::AppHandle,
    fixture: &Fixture,
    run: RunIdentity<'_>,
    rect: BrowserRect,
    heartbeat: &ViewportHeartbeat,
) -> anyhow::Result<String> {
    heartbeat
        .pause_before_signal(&fixture.state, &fixture.state.viewer_detached)
        .await?;
    fixture
        .state
        .wait(
            &fixture.state.session_reopen.closed,
            "exact first-session close",
        )
        .await?;
    poll_closed_gap(app, &run, heartbeat).await?;
    fixture
        .state
        .signal(&fixture.state.session_reopen.gap_observed);
    fixture
        .state
        .wait(
            &fixture.state.session_reopen.observed,
            "Core close and fresh-session snapshot",
        )
        .await?;
    let scope = fixture.state.session_reopen.scope()?;
    ensure!(
        scope.session_id != run.first_session,
        "reopen reused the first session"
    );
    let state = app.state::<AppState>();
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    let tab = loop {
        // Use the controller's real watch-then-snapshot ordering and operation lock.
        context(app, &run).await?;
        let attached = {
            let registry = state
                .browser
                .lock()
                .map_err(|error| anyhow::anyhow!(error.message))?;
            registry
                .tabs
                .iter()
                .find(|tab| {
                    tab.scope == SPACE
                        && tab.agent_run_id.as_deref() == Some(run.run_id)
                        && tab.dto.session_id == scope.session_id
                        && registry.selected.get(SPACE) == Some(&tab.dto.id)
                })
                .map(|tab| tab.dto.id.clone())
        };
        if let Some(tab) = attached {
            break tab;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Desktop never attached the fresh same-run session"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    ensure!(
        tab != run.first_tab,
        "Desktop reused the first native attachment"
    );
    let view = remote(app, &tab)?;
    ensure!(
        view.session_id() == scope.session_id && !view.human_control_available(),
        "reopened viewer changed session or admitted human input"
    );
    heartbeat.resume(&tab, rect).await?;
    accepted_frame(
        app,
        &tab,
        &view,
        rect,
        ExpectedFrame {
            title: TITLE,
            url: &format!("{}/reopened", fixture.origin),
            center_bgra: [204, 232, 255, 255],
            core_target: Some(&scope.target),
            previous: None,
            after: None,
        },
    )
    .await?;
    windows::selected_tab(app, &tab, &view, rect, [204, 232, 255, 255]).await?;
    heartbeat
        .pause_before_signal(&fixture.state, &fixture.state.session_reopen.presented)
        .await?;
    println!(
        "PASS same Core run polled the closed-A gap three times, freshly attached distinct B, and displayed B's exact authenticated frame"
    );
    Ok(tab)
}

async fn poll_closed_gap(
    app: &tauri::AppHandle,
    run: &RunIdentity<'_>,
    heartbeat: &ViewportHeartbeat,
) -> anyhow::Result<()> {
    // A's final trusted pulse was acknowledged before Core could close it.
    // The paused owner stays alive while ordinary context polling observes the
    // closed channel and exceeds the recovery budget without a replacement.
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        heartbeat.check()?;
        if closed_context(app, run).await? {
            break;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Desktop did not observe the exact first-session channel closure"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for _ in 0..3 {
        heartbeat.check()?;
        ensure!(
            closed_context(app, run).await?,
            "missing replacement changed the exact first-session failure"
        );
    }
    Ok(())
}

async fn closed_context(app: &tauri::AppHandle, run: &RunIdentity<'_>) -> anyhow::Result<bool> {
    let snapshot = context(app, run).await?;
    let tab = snapshot
        .tabs
        .iter()
        .find(|tab| tab.id == run.first_tab)
        .context("missing replacement discarded the first-session viewer")?;
    ensure!(
        snapshot.selected_tab_id.as_deref() == Some(run.first_tab)
            && tab.session_id == run.first_session
            && tab.control == "agent",
        "missing replacement changed the first-session identity or selection"
    );
    Ok(tab.error.as_deref() == Some(crate::browser::registry::CRASHED_MESSAGE))
}

async fn context(
    app: &tauri::AppHandle,
    run: &RunIdentity<'_>,
) -> anyhow::Result<crate::browser::dto::BrowserSnapshotDto> {
    let caller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    crate::browser::commands::browser_context(
        app.clone(),
        caller,
        app.state::<AppState>(),
        Some(run.run_id.into()),
        Some(run.conversation_id.into()),
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.message))
}

#[cfg(test)]
mod tests;
