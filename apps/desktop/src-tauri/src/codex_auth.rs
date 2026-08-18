//! Operator-only ChatGPT/Codex sign-in commands for Managed Local.

use colossus_codex_auth::{
    CodexAuthError, CodexAuthStore, CodexCliAction, run_codex_cli_with_environment,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::dto::CommandErrorDto;

static CODEX_AUTH_OPERATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct DesktopCodexAuthStore {
    store: CodexAuthStore,
    home: PathBuf,
}

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
    let Ok(store) = desktop_codex_auth_store() else {
        return unavailable_status();
    };
    let loaded = store.store.load();
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

fn preflight_environment_store(
    action: CodexCliAction,
) -> Result<DesktopCodexAuthStore, CommandErrorDto> {
    let store = desktop_codex_auth_store().map_err(|_| preflight_error(action))?;
    verify_account_precondition(&store.store, action)?;
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
    store: &DesktopCodexAuthStore,
    executable: &Path,
    action: CodexCliAction,
) -> Result<CodexAuthStatusDto, CommandErrorDto> {
    // Repeat the preflight immediately before spawning. The native confirmation can
    // remain open while another process changes the Codex-owned credential file.
    verify_account_precondition(&store.store, action)?;
    run_codex_cli_with_environment(executable, action, [("CODEX_HOME", store.home.as_os_str())])
        .await
        .map_err(|_| cli_operation_error(action))?;
    verify_account_postcondition(&store.store, action)
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
                    unsafe_store_message(),
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
    auth_error(operation_error_code(action), unsafe_store_message(), false)
}

fn unsafe_store_message() -> &'static str {
    "The official Codex credential store is unavailable or unsafe. Desktop uses a private CODEX_HOME under its application storage unless CODEX_HOME is set. If set, CODEX_HOME must be absolute, and CODEX_HOME\\auth.json must be private to your Windows account. For desktop development, rerun scripts\\desktop-dev.ps1 to repair the dev credential ACL."
}

fn cli_operation_error(action: CodexCliAction) -> CommandErrorDto {
    let message = match action {
        CodexCliAction::Login | CodexCliAction::LoginDeviceCode => {
            "The official Codex CLI could not complete ChatGPT sign-in. Set COLOSSUS_CODEX_BIN to a runnable codex.exe or install Codex on PATH and retry."
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
    let store = desktop_codex_auth_store().map_err(|_| {
        auth_error(
            "codex_auth",
            "The official Codex credential store is unavailable. Sign in with ChatGPT and retry.",
            false,
        )
    })?;
    store.store.load().map_err(|_| {
        auth_error(
            "codex_auth",
            "ChatGPT sign-in is unavailable or expired. Sign in with ChatGPT and retry.",
            false,
        )
    })?;
    Ok(store.store.path().to_path_buf())
}

fn desktop_codex_auth_store() -> Result<DesktopCodexAuthStore, CommandErrorDto> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|home| !home.is_empty()) {
        let home = PathBuf::from(home);
        let store = CodexAuthStore::from_environment().map_err(|_| {
            auth_error(
                "codex_auth",
                "The official Codex credential store is unavailable. CODEX_HOME must be absolute.",
                false,
            )
        })?;
        return Ok(DesktopCodexAuthStore { store, home });
    }

    let settings_store = crate::desktop_settings::SettingsStore::open_application()?;
    let home = settings_store.codex_auth_home()?;
    Ok(DesktopCodexAuthStore {
        store: CodexAuthStore::at_path(home.join("auth.json")),
        home,
    })
}

fn codex_executable() -> PathBuf {
    if let Some(path) = std::env::var_os("COLOSSUS_CODEX_BIN") {
        let path = PathBuf::from(path);
        if path.is_absolute() && path.is_file() {
            return path;
        }
    }
    if let Some(path) = codex_executable_from_path() {
        return path;
    }
    #[cfg(windows)]
    if let Some(path) = local_openai_codex_executable() {
        return path;
    }
    #[cfg(target_os = "macos")]
    for candidate in ["/opt/homebrew/bin/codex", "/usr/local/bin/codex"] {
        if Path::new(candidate).is_file() {
            return PathBuf::from(candidate);
        }
    }
    PathBuf::from(codex_executable_name())
}

fn codex_executable_from_path() -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join(codex_executable_name());
            if candidate.is_file() && !protected_windows_app_package_path(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn local_openai_codex_executable() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
        .join("OpenAI")
        .join("Codex")
        .join("bin");
    local_openai_codex_executable_in(&root)
}

#[cfg(windows)]
fn local_openai_codex_executable_in(root: &Path) -> Option<PathBuf> {
    let mut candidates = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path().join(codex_executable_name());
            let modified = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()?;
            path.is_file().then_some((modified, path))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(left, _), (right, _)| right.cmp(left));
    candidates.into_iter().map(|(_, path)| path).next()
}

#[cfg(windows)]
fn protected_windows_app_package_path(path: &Path) -> bool {
    path.to_string_lossy()
        .to_ascii_lowercase()
        .contains(r"\program files\windowsapps\")
}

#[cfg(not(windows))]
fn protected_windows_app_package_path(_path: &Path) -> bool {
    false
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
                "#!/bin/sh\nif [ \"${{1:-}}\" != \"-c\" ]; then exit 64; fi\nprintf '%s' \"$CODEX_HOME\" > {}\n{mutation}\nexit 0\n",
                shell_path(marker)
            ),
        )
        .expect("write fake Codex");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("set fake Codex mode");
        executable
    }

    #[cfg(unix)]
    fn desktop_store_at_path(path: &Path) -> DesktopCodexAuthStore {
        DesktopCodexAuthStore {
            store: CodexAuthStore::at_path(path),
            home: path.parent().expect("auth parent").to_path_buf(),
        }
    }

    #[test]
    fn fallback_executable_is_fixed_and_argument_free() {
        assert!(matches!(codex_executable_name(), "codex" | "codex.exe"));
        assert!(!codex_executable_name().contains(char::is_whitespace));
    }

    #[test]
    fn login_cli_error_names_explicit_codex_binary_override() {
        let error = cli_operation_error(CodexCliAction::Login);
        assert_eq!(error.code, "codex_login_failed");
        assert!(error.message.contains("COLOSSUS_CODEX_BIN"));
        assert!(error.message.contains("codex.exe"));
    }

    #[test]
    fn unsafe_codex_store_error_names_private_auth_file_repair() {
        let error = preflight_error(CodexCliAction::Login);
        assert_eq!(error.code, "codex_login_failed");
        assert!(error.message.contains("CODEX_HOME\\auth.json"));
        assert!(error.message.contains("private"));
        assert!(error.message.contains("scripts\\desktop-dev.ps1"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_codex_resolution_skips_protected_package_aliases() {
        assert!(protected_windows_app_package_path(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__test\app\resources\codex.exe"
        )));
        assert!(!protected_windows_app_package_path(Path::new(
            r"C:\Users\tester\AppData\Local\OpenAI\Codex\bin\abc\codex.exe"
        )));

        let root = tempfile::tempdir().expect("codex bin root");
        let directory = root.path().join("abc");
        std::fs::create_dir_all(&directory).expect("codex version directory");
        let executable = directory.join("codex.exe");
        std::fs::write(&executable, b"test").expect("codex executable placeholder");

        assert_eq!(
            local_openai_codex_executable_in(root.path()),
            Some(executable)
        );
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
            let store = desktop_store_at_path(&auth);
            let error = run_verified_account_operation(&store, &executable, action)
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
            let store = desktop_store_at_path(&auth);
            let status = run_verified_account_operation(&store, &executable, action)
                .await
                .expect("usable auth passes");

            assert_eq!(status.state, CodexAuthStateDto::SignedIn);
            assert!(marker.is_file());
            assert_eq!(
                fs::read_to_string(&marker).expect("marker content"),
                auth.parent().expect("auth parent").to_string_lossy()
            );
        }

        let root = tempfile::tempdir().expect("test root");
        let auth = root.path().join("missing-codex-home/auth.json");
        let marker = root.path().join("codex-invoked");
        let executable = fake_codex(root.path(), &marker, "");
        let store = desktop_store_at_path(&auth);
        let error = run_verified_account_operation(&store, &executable, CodexCliAction::Login)
            .await
            .expect_err("successful CLI without auth must fail login");
        assert_eq!(error.code, "codex_login_failed");
        assert!(marker.is_file(), "login should reach its postcondition");

        let status_marker = root.path().join("status-invoked");
        let status_executable = fake_codex(root.path(), &status_marker, "");
        let status_store = desktop_store_at_path(&auth);
        let error = run_verified_account_operation(
            &status_store,
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
        let remaining_store = desktop_store_at_path(&remaining_auth);
        let error = run_verified_account_operation(&remaining_store, &noop, CodexCliAction::Logout)
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
        let unsafe_store = desktop_store_at_path(&unsafe_auth);
        let error = run_verified_account_operation(&unsafe_store, &chmod, CodexCliAction::Logout)
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
        let removed_store = desktop_store_at_path(&removed_auth);
        let status =
            run_verified_account_operation(&removed_store, &removal_cli, CodexCliAction::Logout)
                .await
                .expect("removed auth verifies logout");
        assert_eq!(status.state, CodexAuthStateDto::SignedOut);
        assert!(!removed_auth.exists());
    }
}
