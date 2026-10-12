//! Exact Core-selected tabs presented through the existing native compositor.
use super::*;

pub(super) const PAGE: &[u8] = b"<!doctype html><html><head><title>Core owned second tab</title></head><body style=\"background:#fff3c4\"><h1>Core owned second tab</h1><p>Core opened and selected this exact owned tab.</p></body></html>";
pub(super) fn enabled() -> anyhow::Result<bool> {
    match std::env::var_os("COLOSSUS_BROWSER_ACCEPTANCE_OWNED_TABS") {
        None => Ok(false),
        Some(value) if value == "1" => Ok(true),
        _ => anyhow::bail!("owned-tab checkpoint requires the exact opt-in value 1"),
    }
}

#[cfg(test)]
#[path = "owned_tabs_tests.rs"]
mod tests;
async fn acknowledged(
    state: &FixtureState,
    ordinal: usize,
    target: BrowserTarget,
) -> anyhow::Result<()> {
    *state.tab_event.lock().unwrap() = Some((ordinal, target));
    state.changed.notify_waiters();
    tokio::time::timeout(state.model_timeout, async {
        while state.tabs_ack.load(Ordering::Acquire) < ordinal {
            let notified = state.changed.notified();
            state.check()?;
            if state.tabs_ack.load(Ordering::Acquire) < ordinal {
                notified.await;
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("Core selected tab lacked a native presentation acknowledgement")?
}
pub(super) async fn response(
    phase: usize,
    body: &Value,
    origin: &str,
    state: &FixtureState,
) -> anyhow::Result<Value> {
    let scope = state
        .target
        .lock()
        .unwrap()
        .clone()
        .context("owned-tab scope absent")?;
    let base =
        json!({"session_id":scope["session_id"],"control_generation":scope["control_generation"]});
    match phase {
        6 => {
            state
                .wait(&state.tabs_begin, "owned Core tab acceptance")
                .await?;
            let mut args = base;
            args["url"] = json!(format!("{origin}/tab-second"));
            Ok(model_tool(phase, &alias(body, "browser.tab.open")?, &args))
        }
        7 => {
            let opened = tool_result(body, "fixture-6")?;
            let target = browser_target(&opened["tab"])?;
            let original = state
                .navigated_target
                .lock()
                .unwrap()
                .clone()
                .context("original owned tab absent")?;
            ensure!(
                opened["session_id"] == scope["session_id"] && target.tab_id != original.tab_id,
                "Core did not open a distinct owned tab in the exact session"
            );
            *state.second_tab.lock().unwrap() = Some(target.clone());
            acknowledged(state, 1, target).await?;
            let mut args = base;
            args["tab_id"] = json!(original.tab_id);
            Ok(model_tool(
                phase,
                &alias(body, "browser.tab.select")?,
                &args,
            ))
        }
        8 => {
            let selected = tool_result(body, "fixture-7")?;
            let target = browser_target(&selected["tab"])?;
            ensure!(
                Some(&target) == state.navigated_target.lock().unwrap().as_ref()
                    && selected["session_id"] == scope["session_id"],
                "Core selected a different original tab"
            );
            acknowledged(state, 2, target.clone()).await?;
            let mut args = base;
            args["tab_id"] = json!(target.tab_id);
            Ok(model_tool(phase, &alias(body, "browser.tab.close")?, &args))
        }
        9 => {
            let closed = tool_result(body, "fixture-8")?;
            ensure!(
                browser_target(&closed["tab"])?
                    == state
                        .navigated_target
                        .lock()
                        .unwrap()
                        .clone()
                        .context("closed original tab absent")?
                    && closed["session_id"] == scope["session_id"],
                "Core close acknowledged the wrong tab"
            );
            Ok(model_tool(
                phase,
                &alias(body, "browser.tabs")?,
                &json!({"session_id":scope["session_id"]}),
            ))
        }
        10 | 11 => finish(phase, body, state, &scope, base).await,
        _ => anyhow::bail!("unreviewed owned-tab fixture phase"),
    }
}
pub(super) async fn checkpoint(
    app: &tauri::AppHandle,
    id: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    fixture: &Fixture,
) -> anyhow::Result<()> {
    fixture.state.signal(&fixture.state.tabs_begin);
    let mut previous = view.acceptance_probe().await?;
    let initial = previous
        .frame
        .context("owned tabs lacked original agent frame")?
        .lease;
    for ordinal in 1..=3 {
        let target = tokio::time::timeout(PAGE_TIMEOUT, async {
            loop {
                let notified = fixture.state.changed.notified();
                fixture.state.check()?;
                if let Some((actual, target)) = fixture.state.tab_event.lock().unwrap().as_ref()
                    && *actual == ordinal
                {
                    return Ok::<BrowserTarget, anyhow::Error>(target.clone());
                }
                notified.await;
            }
        })
        .await
        .context("Core owned-tab result absent")??;
        let second = ordinal != 2;
        let path = if second { "/tab-second" } else { "/agent" };
        let probe = accepted_frame(
            app,
            id,
            view,
            rect,
            ExpectedFrame {
                title: if second {
                    "Core owned second tab"
                } else {
                    "Agent controlled page"
                },
                url: &format!("{}{path}", fixture.origin),
                center_bgra: if second {
                    [196, 243, 255, 255]
                } else {
                    [216, 247, 217, 255]
                },
                core_target: Some(&target),
                previous: None,
                after: None,
            },
        )
        .await?;
        let frame = probe
            .frame
            .context("selected Core tab lacked native frame")?
            .lease;
        let old = previous
            .frame
            .context("previous selected Core tab frame absent")?
            .lease;
        ensure!(
            frame.tab != old.tab
                && frame.control_generation == initial.control_generation
                && frame.session_generation == initial.session_generation
                && frame.viewport_generation > old.viewport_generation
                && !view.human_control_available(),
            "owned-tab retarget changed writer/session or reused its prior viewport"
        );
        println!(
            "PASS Core owned tab checkpoint ordinal={ordinal} changed native tab under the same writer with a fresh authenticated frame"
        );
        previous = probe;
        fixture.state.tabs_ack.store(ordinal, Ordering::Release);
        fixture.state.changed.notify_waiters();
    }
    Ok(())
}

async fn finish(
    phase: usize,
    body: &Value,
    state: &FixtureState,
    scope: &Value,
    base: Value,
) -> anyhow::Result<Value> {
    match phase {
        10 => {
            let tabs = tool_result(body, "fixture-9")?;
            let list = tabs["tabs"].as_array().context("Core tab list absent")?;
            ensure!(
                list.len() == 1
                    && tabs["session_id"] == scope["session_id"]
                    && tabs["control_generation"] == scope["control_generation"],
                "Core retained an unexpected tab or writer epoch"
            );
            let target = browser_target(&list[0])?;
            ensure!(
                Some(&target) == state.second_tab.lock().unwrap().as_ref()
                    && tabs["selected_tab_id"] == list[0]["tab_id"],
                "Core selected a foreign or closed tab"
            );
            acknowledged(state, 3, target).await?;
            state
                .wait(
                    &state.viewer_detached,
                    "native viewer detachment after owned tabs",
                )
                .await?;
            Ok(model_tool(phase, &alias(body, "browser.close")?, &base))
        }
        11 => {
            let closed = tool_result(body, "fixture-10")?;
            ensure!(
                closed["closed"] == true && closed["session_id"] == scope["session_id"],
                "Core did not close the exact owned tab session"
            );
            Ok(
                json!({"id":"fixture-complete","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Mac contained browser tabs diagnostic complete."},"finish_reason":"stop"}]}),
            )
        }
        _ => anyhow::bail!("unreviewed owned-tab close phase"),
    }
}
