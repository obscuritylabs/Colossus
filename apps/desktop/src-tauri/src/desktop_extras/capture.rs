use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_sdk::{ArtifactPurpose, IdempotencyKey, UploadArtifactRequest};
use sha2::{Digest as _, Sha256};
use tauri::Emitter as _;

#[cfg_attr(
    not(windows),
    expect(
        clippy::unnecessary_wraps,
        reason = "The shared adapter preserves fallible Windows shortcut registration."
    )
)]
pub(super) fn register(
    app: &AppHandle,
    old: Option<&ExtrasPreferences>,
    value: &ExtrasPreferences,
) -> Result<(), CommandErrorDto> {
    #[cfg(windows)]
    {
        use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, ShortcutState};
        if old.is_some_and(|p| {
            p.appshots_enabled == value.appshots_enabled
                && p.appshot_shortcut == value.appshot_shortcut
        }) {
            return Ok(());
        }
        if let Some(previous) = old.filter(|p| p.appshots_enabled) {
            app.global_shortcut()
                .unregister(previous.appshot_shortcut.as_str())
                .map_err(|_| error("The previous App Shot shortcut could not be released."))?;
        }
        if value.appshots_enabled
            && app
                .global_shortcut()
                .on_shortcut(value.appshot_shortcut.as_str(), |app, _, event| {
                    if event.state() == ShortcutState::Pressed {
                        start(app);
                    }
                })
                .is_err()
        {
            if let Some(previous) = old.filter(|p| p.appshots_enabled) {
                let _ = app.global_shortcut().on_shortcut(
                    previous.appshot_shortcut.as_str(),
                    |app, _, event| {
                        if event.state() == ShortcutState::Pressed {
                            start(app);
                        }
                    },
                );
            }
            return Err(error(
                "That shortcut is unavailable or is already used by another app. Choose a different shortcut.",
            ));
        }
    }
    #[cfg(not(windows))]
    let _ = (app, old, value);
    Ok(())
}

#[cfg(windows)]
fn start(app: &AppHandle) {
    let state = app.state::<ExtrasState>();
    if state.capturing.swap(true, Ordering::AcqRel) {
        return;
    }
    let (enabled, include_text) = state.preferences.lock().map_or((false, false), |v| {
        (v.appshots_enabled, v.appshot_include_text)
    });
    if !enabled {
        state.capturing.store(false, Ordering::Release);
        return;
    }
    let generation = state.capture_generation.load(Ordering::Acquire);
    let id = format!("shot_{}", uuid::Uuid::now_v7().simple());
    if let Ok(mut active) = state.active_capture.lock() {
        *active = Some(id.clone());
    } else {
        state.capturing.store(false, Ordering::Release);
        return;
    }
    let _ = app.emit_to("main", "appshot-started", &id);
    // This is called only by the native OS global-hotkey callback. Renderer IPC
    // exposes preview/attach/discard, never a capture primitive or a native handle.
    let Ok(ticket) = colossus_native_computer::prepare_foreground_appshot() else {
        finish_inflight(&state, &id);
        capture_error(
            app,
            &id,
            "Choose a visible, ordinary-user app window before taking an App Shot.",
        );
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result =
            tauri::async_runtime::spawn_blocking(move || ticket.capture(include_text)).await;
        let state = app.state::<ExtrasState>();
        if state.capture_generation.load(Ordering::Acquire) != generation {
            finish_inflight(&state, &id);
            return;
        }
        if let Ok(Ok(capture)) = result {
            if let Ok(mut pending) = state.pending.lock() {
                if state.capture_generation.load(Ordering::Acquire) != generation {
                    finish_inflight(&state, &id);
                    return;
                }
                pending.clear();
                pending.insert(
                    id.clone(),
                    PendingCapture {
                        image: capture.png,
                        text: capture.text,
                        title: capture.window_title,
                        width: capture.width,
                        height: capture.height,
                        created: Instant::now(),
                    },
                );
                let _ = app.emit_to("main", "appshot-ready", &id);
            }
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
                let _ = main.set_focus();
            }
        } else {
            let message = if matches!(
                result,
                Ok(Err(colossus_ports::ComputerDriverError::OutcomeUnknown))
            ) {
                "App Shot worker shutdown could not be confirmed. Close and reopen Colossus before taking another capture."
            } else {
                "The window changed or could not be captured. Take another App Shot when it is ready."
            };
            capture_error(&app, &id, message);
        }
        finish_inflight(&state, &id);
    });
}

pub(super) fn cancel_inflight(state: &ExtrasState, id: &str) -> Result<(), CommandErrorDto> {
    let mut active = state
        .active_capture
        .lock()
        .map_err(|_| error("Capture cancellation is unavailable."))?;
    if active.as_deref() == Some(id) {
        state.capture_generation.fetch_add(1, Ordering::AcqRel);
        *active = None;
    }
    Ok(())
}
#[cfg(windows)]
fn finish_inflight(state: &ExtrasState, id: &str) {
    if let Ok(mut active) = state.active_capture.lock()
        && active.as_deref() == Some(id)
    {
        *active = None;
    }
    state.capturing.store(false, Ordering::Release);
}

fn capture_error(app: &AppHandle, capture_id: &str, message: &str) {
    let _ = app.emit_to(
        "main",
        "appshot-error",
        serde_json::json!({"captureId":capture_id,"message":message}),
    );
}

pub(super) fn expire_pending(
    pending: &mut BTreeMap<String, PendingCapture>,
    now: Instant,
) -> Vec<String> {
    let expired = pending
        .iter()
        .filter(|(_, value)| {
            now.saturating_duration_since(value.created) >= std::time::Duration::from_mins(10)
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in &expired {
        pending.remove(id);
    }
    expired
}

pub(super) fn start_expiry(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if app.get_webview_window("main").is_none() {
                break;
            }
            let expired = if let Ok(mut pending) = app.state::<ExtrasState>().pending.lock() {
                expire_pending(&mut pending, Instant::now())
            } else {
                Vec::new()
            };
            for id in expired {
                capture_error(
                    &app,
                    &id,
                    "This App Shot preview expired. Capture the window again to attach it.",
                );
            }
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppshotPreview {
    capture_id: String,
    window_title: String,
    width: u32,
    height: u32,
    image_data_url: String,
    text: Option<String>,
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects owned handles and deserializes command values by value."
)]
#[tauri::command(rename_all = "camelCase")]
pub(crate) fn appshot_preview(
    window: WebviewWindow,
    state: State<'_, ExtrasState>,
    capture_id: String,
) -> Result<AppshotPreview, CommandErrorDto> {
    main_window(&window)?;
    let mut pending = state
        .pending
        .lock()
        .map_err(|_| error("The capture preview is busy."))?;
    pending.retain(|_, v| v.created.elapsed() < std::time::Duration::from_mins(10));
    let value = pending
        .get(&capture_id)
        .ok_or_else(|| error("This App Shot expired. Capture the window again."))?;
    Ok(AppshotPreview {
        capture_id,
        window_title: value.title.clone(),
        width: value.width,
        height: value.height,
        image_data_url: format!("data:image/png;base64,{}", STANDARD.encode(&value.image)),
        text: value.text.clone(),
    })
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects owned handles and deserializes command values by value."
)]
#[tauri::command(rename_all = "camelCase")]
pub(crate) fn discard_appshot(
    window: WebviewWindow,
    state: State<'_, ExtrasState>,
    capture_id: String,
) -> Result<(), CommandErrorDto> {
    main_window(&window)?;
    cancel_inflight(&state, &capture_id)?;
    state
        .pending
        .lock()
        .map_err(|_| error("Capture preview is busy."))?
        .remove(&capture_id);
    Ok(())
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects owned handles and deserializes command values by value."
)]
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn attach_appshot(
    window: WebviewWindow,
    state: State<'_, crate::state::AppState>,
    extras: State<'_, ExtrasState>,
    capture_id: String,
    target_id: String,
    include_text: bool,
) -> Result<Vec<crate::dto::ArtifactReferenceDto>, CommandErrorDto> {
    main_window(&window)?;
    let (image, text) = {
        let pending = extras
            .pending
            .lock()
            .map_err(|_| error("Capture preview is busy."))?;
        let value = pending
            .get(&capture_id)
            .filter(|v| v.created.elapsed() < std::time::Duration::from_mins(10))
            .ok_or_else(|| error("This App Shot expired."))?;
        (
            value.image.clone(),
            if include_text {
                value.text.clone()
            } else {
                None
            },
        )
    };
    let lease = state.selected_target(&target_id).await.ok_or_else(|| {
        error("The selected chat destination changed. Review it before attaching.")
    })?;
    let _slot = crate::commands::unary_slot(&lease.target)?;
    let mut parts = vec![(format!("{capture_id}.png"), "image/png", image)];
    if let Some(text) = text {
        parts.push((format!("{capture_id}.txt"), "text/plain", text.into_bytes()));
    }
    let mut references = Vec::new();
    for (file_name, media_type, bytes) in parts {
        let key = format!(
            "appshot-{:x}",
            Sha256::digest([target_id.as_bytes(), file_name.as_bytes(), bytes.as_slice()].concat())
        );
        let key = IdempotencyKey::new(key)
            .map_err(|_| error("The App Shot upload identity is invalid."))?;
        let artifact = lease
            .target
            .client
            .upload_artifact(UploadArtifactRequest {
                file_name,
                media_type: media_type.into(),
                purpose: ArtifactPurpose::RunInput,
                bytes,
                idempotency_key: key,
            })
            .await
            .map_err(CommandErrorDto::from_api)?;
        references.push(artifact.into());
    }
    extras
        .pending
        .lock()
        .map_err(|_| error("Capture preview is busy."))?
        .remove(&capture_id);
    Ok(references)
}
