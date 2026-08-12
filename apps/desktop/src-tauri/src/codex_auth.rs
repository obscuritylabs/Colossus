//! Operator-only ChatGPT/Codex sign-in commands for Managed Local.

use colossus_codex_auth::{CodexAuthError, CodexAuthStore, CodexCliAction, run_codex_cli};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::dto::CommandErrorDto;

static CODEX_AUTH_OPERATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CodexAuthStateDto {
    SignedIn,
    SignedOut,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexAuthStatusDto {
    pub(crate) state: CodexAuthStateDto,
    pub(crate) message: String,
}

#[tauri::command]
pub(crate) fn codex_auth_status() -> CodexAuthStatusDto {
    current_status()
}

#[tauri::command]
pub(crate) async fn codex_auth_login(
    app: AppHandle,
) -> Result<CodexAuthStatusDto, CommandErrorDto> {
    let _guard = CODEX_AUTH_OPERATION.try_lock().map_err(|_| {
        CommandErrorDto::busy("Another ChatGPT sign-in operation is already active.")
    })?;
    let store = preflight_environment_store(CodexCliAction::Login)?;
    if !confirm_account_operation(
        &app,
        "Sign in with ChatGPT",
        "Colossus will start the official Codex CLI and open its ChatGPT sign-in flow. The resulting token remains in the Codex credential store and never enters the Desktop WebView.\n\nContinue?",
        "Continue",
    )
    .await?
    {
        return Err(auth_error(
            "codex_login_cancelled",
            "ChatGPT sign-in was cancelled.",
            false,
        ));
    }
    let executable = codex_executable();
    run_verified_account_operation(&store, &executable, CodexCliAction::Login).await
}

#[tauri::command]
pub(crate) async fn codex_auth_logout(
    app: AppHandle,
) -> Result<CodexAuthStatusDto, CommandErrorDto> {
    let _guard = CODEX_AUTH_OPERATION.try_lock().map_err(|_| {
        CommandErrorDto::busy("Another ChatGPT sign-in operation is already active.")
    })?;
    let store = preflight_environment_store(CodexCliAction::Logout)?;
    if !confirm_account_operation(
        &app,
        "Sign out of ChatGPT",
        "This asks the official Codex CLI to remove its current ChatGPT sign-in. Managed Local Codex runs will stop working until you sign in again.\n\nContinue?",
        "Sign out",
    )
    .await?
    {
        return Err(auth_error(
            "codex_logout_cancelled",
            "ChatGPT sign-out was cancelled.",
            false,
        ));
    }
    let executable = codex_executable();
    run_verified_account_operation(&store, &executable, CodexCliAction::Logout).await
}

pub(crate) fn current_status() -> CodexAuthStatusDto {
    let loaded = CodexAuthStore::from_environment().and_then(|store| store.load());
    status_from_load(&loaded)
}

fn status_from_load(
    loaded: &Result<colossus_codex_auth::CodexAuthorization, CodexAuthError>,
) -> CodexAuthStatusDto {
    match loaded {
        Ok(_) => signed_in_status(),
        Err(CodexAuthError::Unavailable(_)) => signed_out_status(),
        Err(CodexAuthError::Storage(_) | CodexAuthError::Cli(_)) => unavailable_status(),
    }
}

fn signed_in_status() -> CodexAuthStatusDto {
    CodexAuthStatusDto {
        state: CodexAuthStateDto::SignedIn,
        message: "Signed in with ChatGPT through the official Codex credential store.".into(),
    }
}

fn signed_out_status() -> CodexAuthStatusDto {
    CodexAuthStatusDto {
        state: CodexAuthStateDto::SignedOut,
        message: "Sign in with ChatGPT to use the Codex subscription provider.".into(),
    }
}

fn unavailable_status() -> CodexAuthStatusDto {
    CodexAuthStatusDto {
        state: CodexAuthStateDto::Unavailable,
        message: "The official Codex credential store is unavailable or unsafe.".into(),
    }
}

fn preflight_environment_store(action: CodexCliAction) -> Result<CodexAuthStore, CommandErrorDto> {
    let store = CodexAuthStore::from_environment().map_err(|_| preflight_error(action))?;
    verify_account_precondition(&store, action)?;
    Ok(store)
}

fn verify_account_precondition(
    store: &CodexAuthStore,
    action: CodexCliAction,
) -> Result<(), CommandErrorDto> {
    match (action, store.load()) {
        (CodexCliAction::Status, Ok(_))
        | (
            CodexCliAction::Login | CodexCliAction::LoginDeviceCode | CodexCliAction::Logout,
            Ok(_) | Err(CodexAuthError::Unavailable(_)),
        ) => Ok(()),
        (CodexCliAction::Status, Err(CodexAuthError::Unavailable(_))) => Err(auth_error(
            "codex_status_failed",
            "No runtime-usable file-backed ChatGPT credential is available. Sign in with ChatGPT and retry.",
            false,
        )),
        (_, Err(CodexAuthError::Storage(_) | CodexAuthError::Cli(_))) => {
            Err(preflight_error(action))
        }
    }
}

async fn run_verified_account_operation(
    store: &CodexAuthStore,
    executable: &Path,
    action: CodexCliAction,
) -> Result<CodexAuthStatusDto, CommandErrorDto> {
    // Repeat the preflight immediately before spawning. The native confirmation can
    // remain open while another process changes the Codex-owned credential file.
    verify_account_precondition(store, action)?;
    run_codex_cli(executable, action)
        .await
        .map_err(|_| cli_operation_error(action))?;
    verify_account_postcondition(store, action)
}

fn verify_account_postcondition(
    store: &CodexAuthStore,
    action: CodexCliAction,
) -> Result<CodexAuthStatusDto, CommandErrorDto> {
    let loaded = store.load();
    match action {
        CodexCliAction::Login | CodexCliAction::LoginDeviceCode | CodexCliAction::Status => {
            match loaded {
                Ok(_) => Ok(signed_in_status()),
                Err(CodexAuthError::Unavailable(_)) => Err(auth_error(
                    operation_error_code(action),
                    "The official Codex CLI completed, but no usable file-backed ChatGPT credential is available.",
                    true,
                )),
                Err(CodexAuthError::Storage(_) | CodexAuthError::Cli(_)) => Err(auth_error(
                    operation_error_code(action),
                    "The official Codex CLI completed, but its credential store could not be validated safely.",
                    false,
                )),
            }
        }
        CodexCliAction::Logout => match loaded {
            Err(CodexAuthError::Unavailable(_)) => Ok(signed_out_status()),
            Err(CodexAuthError::Storage(_) | CodexAuthError::Cli(_)) => Err(auth_error(
                "codex_logout_failed",
                "The official Codex CLI completed ChatGPT sign-out, but credential removal could not be verified safely.",
                false,
            )),
            Ok(_) => Err(auth_error(
                "codex_logout_failed",
                "The official Codex CLI completed ChatGPT sign-out, but a usable credential remains.",
                true,
            )),
        },
    }
}

fn preflight_error(action: CodexCliAction) -> CommandErrorDto {
    auth_error(
        operation_error_code(action),
        "The official Codex credential store is unavailable or unsafe. CODEX_HOME must be absolute and the file-backed credential must be private.",
        false,
    )
}

fn cli_operation_error(action: CodexCliAction) -> CommandErrorDto {
    let message = match action {
        CodexCliAction::Login | CodexCliAction::LoginDeviceCode => {
            "The official Codex CLI could not complete ChatGPT sign-in. Install Codex on PATH and retry."
        }
        CodexCliAction::Status => {
            "The official Codex CLI could not validate the current ChatGPT sign-in."
        }
        CodexCliAction::Logout => "The official Codex CLI could not complete ChatGPT sign-out.",
    };
    auth_error(operation_error_code(action), message, true)
}

const fn operation_error_code(action: CodexCliAction) -> &'static str {
    match action {
        CodexCliAction::Login | CodexCliAction::LoginDeviceCode => "codex_login_failed",
        CodexCliAction::Status => "codex_status_failed",
        CodexCliAction::Logout => "codex_logout_failed",
    }
}

pub(crate) fn require_codex_auth_path() -> Result<PathBuf, CommandErrorDto> {
    let store = CodexAuthStore::from_environment().map_err(|_| {
        auth_error(
            "codex_auth",
            "The official Codex credential store is unavailable. Sign in with ChatGPT and retry.",
            false,
        )
    })?;
    store.load().map_err(|_| {
        auth_error(
            "codex_auth",
            "ChatGPT sign-in is unavailable or expired. Sign in with ChatGPT and retry.",
            false,
        )
    })?;
    Ok(store.path().to_path_buf())
}

fn codex_executable() -> PathBuf {
    if let Some(path) = std::env::var_os("COLOSSUS_CODEX_BIN") {
        let path = PathBuf::from(path);
        if path.is_absolute() && path.is_file() {
            return path;
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join(codex_executable_name());
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    #[cfg(target_os = "macos")]
    for candidate in ["/opt/homebrew/bin/codex", "/usr/local/bin/codex"] {
        if Path::new(candidate).is_file() {
            return PathBuf::from(candidate);
        }
    }
    PathBuf::from(codex_executable_name())
}

const fn codex_executable_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}

async fn confirm_account_operation(
    app: &AppHandle,
    title: &'static str,
    message: &'static str,
    confirm_label: &'static str,
) -> Result<bool, CommandErrorDto> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title(title)
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(
                confirm_label.into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        auth_error(
            "codex_auth_confirmation",
            "The native ChatGPT account confirmation could not be opened.",
            true,
        )
    })
}

fn auth_error(code: &str, message: &str, retryable: bool) -> CommandErrorDto {
    CommandErrorDto::local_sanitized(code, message, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use serde_json::json;
    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt as _};

    #[cfg(unix)]
    const TEST_ID_TOKEN: &str = "header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiZGVza3RvcC1jb2RleC10ZXN0LWFjY291bnQifX0.signature";
    #[cfg(unix)]
    const TEST_ACCESS_TOKEN: &str = "header.eyJleHAiOjQxMDI0NDQ4MDB9.signature";

    #[cfg(unix)]
    fn write_auth(path: &Path, mode: u32) {
        fs::create_dir_all(path.parent().expect("auth parent")).expect("create auth parent");
        fs::write(
            path,
            serde_json::to_vec(&json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": TEST_ID_TOKEN,
                    "access_token": TEST_ACCESS_TOKEN,
                    "refresh_token": "desktop-codex-test-refresh-token",
                    "account_id": "desktop-codex-test-account"
                }
            }))
            .expect("serialize auth"),
        )
        .expect("write auth");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set auth mode");
    }

    #[cfg(unix)]
    fn shell_path(path: &Path) -> String {
        format!(
            "'{}'",
            path.to_str()
                .expect("test path is UTF-8")
                .replace('\'', "'\"'\"'")
        )
    }

    #[cfg(unix)]
    fn fake_codex(root: &Path, marker: &Path, mutation: &str) -> PathBuf {
        let executable = root.join(format!("fake-codex-{}", uuid::Uuid::now_v7()));
        fs::write(
            &executable,
            format!(
                "#!/bin/sh\nif [ \"${{1:-}}\" != \"-c\" ]; then exit 64; fi\n: > {}\n{mutation}\nexit 0\n",
                shell_path(marker)
            ),
        )
        .expect("write fake Codex");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("set fake Codex mode");
        executable
    }

    #[test]
    fn fallback_executable_is_fixed_and_argument_free() {
        assert!(matches!(codex_executable_name(), "codex" | "codex.exe"));
        assert!(!codex_executable_name().contains(char::is_whitespace));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unsafe_store_fails_before_any_codex_cli_spawn() {
        for action in [
            CodexCliAction::Login,
            CodexCliAction::Status,
            CodexCliAction::Logout,
        ] {
            let root = tempfile::tempdir().expect("test root");
            let auth = root.path().join("codex-home/auth.json");
            write_auth(&auth, 0o644);
            let marker = root.path().join("codex-invoked");
            let executable = fake_codex(root.path(), &marker, "");
            let error = run_verified_account_operation(
                &CodexAuthStore::at_path(&auth),
                &executable,
                action,
            )
            .await
            .expect_err("unsafe auth must fail");

            assert_eq!(error.code, operation_error_code(action));
            assert!(!marker.exists(), "unsafe auth spawned {action:?}");
            assert!(
                !error
                    .message
                    .contains(root.path().to_string_lossy().as_ref())
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn login_and_status_complete_only_for_signed_in_state() {
        for action in [CodexCliAction::Login, CodexCliAction::Status] {
            let root = tempfile::tempdir().expect("test root");
            let auth = root.path().join("codex-home/auth.json");
            write_auth(&auth, 0o600);
            let marker = root.path().join("codex-invoked");
            let executable = fake_codex(root.path(), &marker, "");
            let status = run_verified_account_operation(
                &CodexAuthStore::at_path(&auth),
                &executable,
                action,
            )
            .await
            .expect("usable auth passes");

            assert_eq!(status.state, CodexAuthStateDto::SignedIn);
            assert!(marker.is_file());
        }

        let root = tempfile::tempdir().expect("test root");
        let auth = root.path().join("missing-codex-home/auth.json");
        let marker = root.path().join("codex-invoked");
        let executable = fake_codex(root.path(), &marker, "");
        let error = run_verified_account_operation(
            &CodexAuthStore::at_path(&auth),
            &executable,
            CodexCliAction::Login,
        )
        .await
        .expect_err("successful CLI without auth must fail login");
        assert_eq!(error.code, "codex_login_failed");
        assert!(marker.is_file(), "login should reach its postcondition");

        let status_marker = root.path().join("status-invoked");
        let status_executable = fake_codex(root.path(), &status_marker, "");
        let error = run_verified_account_operation(
            &CodexAuthStore::at_path(&auth),
            &status_executable,
            CodexCliAction::Status,
        )
        .await
        .expect_err("missing auth must fail status");
        assert_eq!(error.code, "codex_status_failed");
        assert!(!status_marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn logout_requires_verified_unavailable_state() {
        let remaining = tempfile::tempdir().expect("remaining root");
        let remaining_auth = remaining.path().join("codex-home/auth.json");
        write_auth(&remaining_auth, 0o600);
        let remaining_marker = remaining.path().join("codex-invoked");
        let noop = fake_codex(remaining.path(), &remaining_marker, "");
        let error = run_verified_account_operation(
            &CodexAuthStore::at_path(&remaining_auth),
            &noop,
            CodexCliAction::Logout,
        )
        .await
        .expect_err("remaining auth must fail logout");
        assert_eq!(error.code, "codex_logout_failed");
        assert!(error.message.contains("usable credential remains"));

        let unsafe_after = tempfile::tempdir().expect("unsafe root");
        let unsafe_auth = unsafe_after.path().join("codex-home/auth.json");
        write_auth(&unsafe_auth, 0o600);
        let unsafe_marker = unsafe_after.path().join("codex-invoked");
        let chmod = fake_codex(
            unsafe_after.path(),
            &unsafe_marker,
            &format!("/bin/chmod 0644 {}", shell_path(&unsafe_auth)),
        );
        let error = run_verified_account_operation(
            &CodexAuthStore::at_path(&unsafe_auth),
            &chmod,
            CodexCliAction::Logout,
        )
        .await
        .expect_err("unsafe remaining auth must fail logout");
        assert_eq!(error.code, "codex_logout_failed");
        assert!(error.message.contains("could not be verified safely"));

        let removed = tempfile::tempdir().expect("removed root");
        let removed_auth = removed.path().join("codex-home/auth.json");
        write_auth(&removed_auth, 0o600);
        let removed_marker = removed.path().join("codex-invoked");
        let removal_cli = fake_codex(
            removed.path(),
            &removed_marker,
            &format!("/bin/rm -f -- {}", shell_path(&removed_auth)),
        );
        let status = run_verified_account_operation(
            &CodexAuthStore::at_path(&removed_auth),
            &removal_cli,
            CodexCliAction::Logout,
        )
        .await
        .expect("removed auth verifies logout");
        assert_eq!(status.state, CodexAuthStateDto::SignedOut);
        assert!(!removed_auth.exists());
    }
}
