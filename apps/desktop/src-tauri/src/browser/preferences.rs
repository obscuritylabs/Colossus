//! Trusted local browser preferences; guest pages cannot select profiles or paths.

use super::{
    commands::require_controller,
    manager::error,
    profile::{HumanBrowserPreferences, ProfileStore},
};
use crate::{dto::CommandErrorDto, state::AppState};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, Webview};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BrowserPreferencesRequest {
    generation: u64,
    action: PreferencesAction,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PreferencesAction {
    Status,
    Save {
        preferences: HumanBrowserPreferences,
    },
    ClearData,
    Reset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserPreferencesStatus {
    generation: u64,
    available: bool,
    preferences: HumanBrowserPreferences,
    message: &'static str,
}

/// Workspace authority is obtained from `AppState`, never from renderer parameters.
#[tauri::command]
pub(crate) async fn browser_preferences(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    request: BrowserPreferencesRequest,
) -> Result<BrowserPreferencesStatus, CommandErrorDto> {
    require_controller(&caller)?;
    let _operation = state.browser.operation.lock().await;
    state.browser.require_open()?;
    let selected = state.browser_selection().await;
    state.browser.selection_changed(selected.clone());
    let scope = selected
        .clone()
        .ok_or_else(|| error("Select a workspace to configure its browser profile."))?;
    if state.browser.lock()?.generation != request.generation {
        return Err(error(
            "The selected workspace changed. Reopen browser settings.",
        ));
    }
    state.browser.load_human_preferences(&scope)?;
    let store = ProfileStore::open(&scope)?;
    let reset = matches!(&request.action, PreferencesAction::Reset);
    match request.action {
        PreferencesAction::Status => {}
        PreferencesAction::Save { preferences } => {
            preferences.validate()?;
            close_human_tabs(&state, &scope).await?;
            if let Err(failure) = store.write(&preferences) {
                state.browser.lock()?.human_preferences.remove(&scope);
                return Err(failure);
            }
            state
                .browser
                .lock()?
                .human_preferences
                .insert(scope.clone(), preferences);
        }
        PreferencesAction::ClearData | PreferencesAction::Reset => {
            #[cfg(windows)]
            {
                clear_data(&app, &state, &scope, &store).await?;
                if reset {
                    let preferences = HumanBrowserPreferences::default();
                    if let Err(failure) = store.write(&preferences) {
                        state.browser.lock()?.human_preferences.remove(&scope);
                        return Err(failure);
                    }
                    state
                        .browser
                        .lock()?
                        .human_preferences
                        .insert(scope.clone(), preferences);
                }
            }
            #[cfg(not(windows))]
            {
                let _ = reset;
                return Err(error(
                    "Saved browsing-data controls currently require Windows WebView2.",
                ));
            }
        }
    }
    #[cfg(not(windows))]
    let _ = app;
    let registry = state.browser.lock()?;
    Ok(BrowserPreferencesStatus {
        generation: registry.generation,
        available: cfg!(windows),
        preferences: registry
            .human_preferences
            .get(&scope)
            .cloned()
            .unwrap_or_default(),
        message: "Settings apply to human browsing in this workspace. Remember sign-ins or a custom User-Agent explicitly selects Windows WebView2. Agent Chromium sessions remain temporary and separate.",
    })
}

async fn close_human_tabs(state: &AppState, scope: &str) -> Result<(), CommandErrorDto> {
    state.browser.hide_all();
    let ids: Vec<_> = state
        .browser
        .lock()?
        .tabs
        .iter()
        .filter(|tab| {
            tab.scope == scope
                && tab.agent_run_id.is_none()
                && !matches!(tab.dto.control, "agent" | "paused")
        })
        .map(|tab| tab.dto.id.clone())
        .collect();
    for id in ids {
        state.browser.close(scope, &id).await?;
    }
    let mut registry = state.browser.lock()?;
    if registry
        .tabs
        .iter()
        .any(|tab| tab.scope == scope && tab.view.system_view().is_some())
    {
        return Err(error("A human browser tab could not be closed safely."));
    }
    registry.generation = registry.generation.wrapping_add(1);
    Ok(())
}

#[cfg(windows)]
async fn clear_data(
    app: &AppHandle,
    state: &AppState,
    scope: &str,
    store: &ProfileStore,
) -> Result<(), CommandErrorDto> {
    close_human_tabs(state, scope).await?;
    // A clear must still be possible when the profile crossed its admission cap.
    if !state.browser.lock()?.human_profiles.contains_key(scope) {
        if state.browser.lock()?.human_profiles.len() >= super::profile::MAX_HUMAN_PROFILES {
            return Err(error(
                "Restart Colossus to release inactive browser profile leases before clearing another workspace.",
            ));
        }
        let lease = store.lease_for_clear()?;
        state
            .browser
            .lock()?
            .human_profiles
            .insert(scope.to_owned(), lease);
    }
    let directory = state
        .browser
        .lock()?
        .human_profiles
        .get(scope)
        .ok_or_else(|| error("The saved browser profile is unavailable."))?
        .directory_for_clear()?;
    let view = super::guest::create_system(
        app,
        format!("browser-clear-{}", uuid::Uuid::new_v4()),
        directory,
        None,
        colossus_native_browser::NavigationPolicy::default(),
        std::sync::Arc::new(|_| {}),
        HumanBrowserPreferences {
            remember_sign_ins: true,
            user_agent: String::new(),
        },
    )
    .await?;
    let native = view
        .system_view()
        .ok_or_else(|| error("The saved browser profile is unavailable."))?;
    let cleared = colossus_native_browser::clear_site_data(native).await;
    let closed = view.close().await;
    cleared.map_err(|_| error("WebView2 did not confirm that browsing data was cleared. The outcome is unconfirmed; inspect the profile before retrying."))?;
    closed.map_err(|_| {
        error("Browsing data was cleared, but the native controller did not finish closing.")
    })?;
    Ok(())
}
