//! Real frame continuity while a finite Core wait owns the unchanged document.
use super::{
    BrowserTarget, Fixture, FixtureState, RemoteAcceptedFrame, RemoteSurface, ViewportHeartbeat,
    alias, browser_target, respond, same_document_frame_after, tool_result,
};
use anyhow::{Context as _, ensure};
use colossus_sdk::{Colossus, RunUpdateKind, ToolActivityState, WatchRunRequest};
use serde_json::{Value, json};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::net::TcpStream;

const CALL: &str = "fixture-wait";
const TIMEOUT: Duration = Duration::from_secs(8);
// The existing cursor-indicator screen checkpoint runs before Core may wait.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(20);
const TITLE: &str = "Agent wait complete";

#[derive(Default)]
pub(super) struct State {
    timer_ready: AtomicBool,
    release: AtomicBool,
    completed: AtomicBool,
    target: Mutex<Option<BrowserTarget>>,
}

pub(super) fn logical_phase(turn: usize) -> usize {
    turn - usize::from(turn > 4)
}

pub(super) async fn release_response(
    stream: &mut TcpStream,
    state: &FixtureState,
) -> anyhow::Result<()> {
    ensure!(
        !state
            .wait_continuity
            .timer_ready
            .swap(true, Ordering::AcqRel),
        "the Core fixture clicked its wait trigger more than once"
    );
    state.changed.notify_waiters();
    tokio::time::timeout(
        RELEASE_TIMEOUT,
        state.wait(&state.wait_continuity.release, "native wait frames"),
    )
    .await
    .context("same-document wait release exceeded its deadline")??;
    respond(stream, 200, "text/plain", b"release same document").await
}

pub(super) fn model_wait(body: &Value, state: &FixtureState) -> anyhow::Result<Value> {
    let snapshot = tool_result(body, "fixture-3")?;
    let node = snapshot["snapshot"]["nodes"]
        .as_array()
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node["name"] == "Agent wait target")
        })
        .context("fresh Core snapshot did not expose the offscreen wait target")?;
    let target = browser_target(&snapshot["tab"])?;
    let mut args = state
        .target
        .lock()
        .unwrap()
        .clone()
        .context("missing wait scope")?;
    args["tab_id"] = snapshot["tab"]["tab_id"].clone();
    args["document_id"] = snapshot["tab"]["document_id"].clone();
    args["condition"] = json!({"kind":"element_visible", "element":node["element"]});
    args["timeout_ms"] = json!(10_000);
    *state.wait_continuity.target.lock().unwrap() = Some(target);
    let mut response = super::model_tool(4, &alias(body, "browser.wait")?, &args);
    response["choices"][0]["message"]["tool_calls"][0]["id"] = json!(CALL);
    Ok(response)
}

pub(super) fn verify_result(body: &Value, state: &FixtureState) -> anyhow::Result<()> {
    let result = tool_result(body, CALL)?;
    ensure!(
        state.wait_continuity.release.load(Ordering::Acquire)
            && Some(&browser_target(&result["tab"])?)
                == state.wait_continuity.target.lock().unwrap().as_ref()
            && result["tab"]["title"] == TITLE,
        "Core wait completed early or changed its owned document"
    );
    state.signal(&state.wait_continuity.completed);
    Ok(())
}

async fn started(updates: &mut colossus_sdk::RunUpdates) -> anyhow::Result<()> {
    while let Some(update) = updates.next_update().await {
        if let RunUpdateKind::ToolActivity(activity) = update?.update
            && activity.call_id == CALL
        {
            ensure!(
                activity.tool_name == "browser.wait",
                "wait activity named a different tool"
            );
            match activity.state {
                ToolActivityState::Started => return Ok(()),
                ToolActivityState::Requested | ToolActivityState::WaitingApproval => {}
                _ => anyhow::bail!("Core wait ended before its native frame checkpoint"),
            }
        }
    }
    anyhow::bail!("Core feed ended before the wait started")
}

async fn frame(
    view: &RemoteSurface,
    state: &FixtureState,
    previous: Option<RemoteAcceptedFrame>,
) -> anyhow::Result<RemoteAcceptedFrame> {
    let probe = view.acceptance_probe().await?;
    let frame = probe
        .frame
        .context("pending Core wait lost its authenticated native frame")?;
    ensure!(
        probe.visible
            && !probe.page.loading
            && probe.page.title == "Agent click received"
            && Some(&probe.page.target) == state.wait_continuity.target.lock().unwrap().as_ref()
            && frame.lease.control_generation > 0
            && frame.sequence > 0
            && frame.count > 0
            && frame.center_bgra == [255, 241, 231, 255],
        "pending Core wait hid or replaced its unchanged agent page"
    );
    if let Some(previous) = previous {
        ensure!(
            frame.lease == previous.lease && same_document_frame_after(frame, previous),
            "pending Core wait stopped authenticated frame continuity"
        );
    }
    let screen = view.acceptance_windows_screen().await?;
    ensure!(
        screen.foreground
            && screen.visible
            && screen.hit_test
            && screen.center_bgra == [255, 241, 231, 255],
        "pending Core wait blanked its actual native HWND"
    );
    Ok(frame)
}

fn pending(update: RunUpdateKind) -> anyhow::Result<()> {
    match update {
        RunUpdateKind::ToolActivity(activity) if activity.call_id == CALL => {
            ensure!(
                activity.tool_name == "browser.wait"
                    && matches!(activity.state, ToolActivityState::Started),
                "Core wait reached a terminal outcome before native continuity was proved"
            );
        }
        RunUpdateKind::Result(_)
        | RunUpdateKind::Failure { .. }
        | RunUpdateKind::Cancellation(_) => {
            anyhow::bail!("Core run ended before pending-wait native continuity was proved");
        }
        _ => {}
    }
    Ok(())
}

pub(super) async fn checkpoint(
    client: &Colossus,
    run_id: &str,
    view: &RemoteSurface,
    fixture: &Fixture,
    heartbeat: &ViewportHeartbeat,
) -> anyhow::Result<()> {
    tokio::time::timeout(TIMEOUT, async {
        let mut updates = client
            .watch_run(WatchRunRequest {
                run_id: run_id.into(),
                after_sequence: 0,
            })
            .await?;
        started(&mut updates).await?;
        let continuity = async {
            fixture
                .state
                .wait(
                    &fixture.state.wait_continuity.timer_ready,
                    "the page's finite wait timer",
                )
                .await?;
            let mut previous = None;
            for _ in 0..4 {
                heartbeat.check()?;
                fixture.state.check()?;
                ensure!(
                    !fixture.state.wait_continuity.completed.load(Ordering::Acquire),
                    "Core wait completed before frame continuity was proved"
                );
                previous = Some(frame(view, &fixture.state, previous).await?);
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            Ok::<(), anyhow::Error>(())
        };
        tokio::pin!(continuity);
        loop {
            tokio::select! {
                biased;
                update = updates.next_update() => pending(update.context("Core feed ended during its pending wait")??.update)?,
                result = &mut continuity => { result?; break; }
            }
        }
        fixture.state.signal(&fixture.state.wait_continuity.release);
        fixture
            .state
            .wait(
                &fixture.state.wait_continuity.completed,
                "Core's released same-document wait result",
            )
            .await?;
        println!("PASS pending Core browser.wait kept advancing authenticated native frames and visible HWND pixels under the same agent document and writer");
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("same-document Core wait continuity exceeded its deadline")?
}

#[cfg(test)]
mod tests;
