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
struct Reopened {
    session_id: String,
    target: BrowserTarget,
}

#[derive(Default)]
pub(super) struct State {
    observed: AtomicBool,
    presented: AtomicBool,
    scope: Mutex<Option<Reopened>>,
}

impl State {
    fn scope(&self) -> anyhow::Result<Reopened> {
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
    fixture.state.signal(&fixture.state.viewer_detached);
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
        // Inspect observes old-channel retirement just as normal browser_context does.
        state
            .browser
            .snapshot()
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        state
            .browser
            .watch_agent(app, SPACE, run.run_id, run.conversation_id)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
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
    heartbeat.pause().await?;
    fixture
        .state
        .signal(&fixture.state.session_reopen.presented);
    println!(
        "PASS same Core run explicitly closed A, freshly attached distinct B, and displayed B's exact authenticated frame"
    );
    Ok(tab)
}

#[cfg(test)]
mod tests;
