use colossus_codex_auth::{CodexAuthError, CodexAuthStore, CodexCliAction, run_codex_cli};
use std::{
    error::Error,
    io,
    path::{Path, PathBuf},
};

const CODEX_BIN_ENVIRONMENT: &str = "COLOSSUS_CODEX_BIN";

pub(super) async fn run_codex_account_operation(
    executable: &Path,
    action: CodexCliAction,
) -> Result<(), Box<dyn Error>> {
    let executable = resolve_codex_executable(executable);
    let store = CodexAuthStore::from_environment().map_err(codex_store_resolution_error)?;
    verify_codex_account_precondition(&store, action)?;
    run_codex_cli(&executable, action).await?;
    verify_codex_account_state(&store, action)?;
    Ok(())
}

fn resolve_codex_executable(requested: &Path) -> PathBuf {
    if !is_default_codex_executable(requested) {
        return requested.to_path_buf();
    }
    if let Some(path) = std::env::var_os(CODEX_BIN_ENVIRONMENT) {
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
    requested.to_path_buf()
}

fn is_default_codex_executable(path: &Path) -> bool {
    path == Path::new("codex") || path == Path::new(codex_executable_name())
}

fn codex_executable_from_path() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(codex_executable_name()))
            .find(|candidate| candidate.is_file() && !protected_windows_app_package_path(candidate))
    })
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

fn codex_store_resolution_error(error: CodexAuthError) -> io::Error {
    match error {
        CodexAuthError::Storage(_) => io::Error::other(
            "Codex account operation cannot start because CODEX_HOME must be absolute",
        ),
        CodexAuthError::Unavailable(_) | CodexAuthError::Cli(_) => io::Error::other(
            "Codex account operation cannot start because the file-backed credential store could not be resolved",
        ),
    }
}

fn verify_codex_account_precondition(
    store: &CodexAuthStore,
    action: CodexCliAction,
) -> Result<(), io::Error> {
    match (action, store.load()) {
        (CodexCliAction::Status, Ok(_))
        | (
            CodexCliAction::Login | CodexCliAction::LoginDeviceCode | CodexCliAction::Logout,
            Ok(_) | Err(CodexAuthError::Unavailable(_)),
        ) => Ok(()),
        (CodexCliAction::Status, Err(CodexAuthError::Unavailable(_))) => Err(io::Error::other(
            "Codex status cannot start because no runtime-usable file-backed ChatGPT credential is available; run `colossus codex login`",
        )),
        (_, Err(CodexAuthError::Storage(_))) => Err(io::Error::other(format!(
            "Codex {} cannot start because the credential store is unsafe; auth.json must be a private current-user-owned regular non-symlink file on Unix",
            codex_operation_name(action)
        ))),
        (_, Err(CodexAuthError::Cli(_))) => Err(io::Error::other(format!(
            "Codex {} cannot start because Colossus could not validate the file-backed credential state",
            codex_operation_name(action)
        ))),
    }
}

fn verify_codex_account_state(
    store: &CodexAuthStore,
    action: CodexCliAction,
) -> Result<(), io::Error> {
    let loaded = store.load();
    match action {
        CodexCliAction::Login | CodexCliAction::LoginDeviceCode | CodexCliAction::Status => {
            loaded.map(|_| ()).map_err(|error| {
                codex_store_error(
                    action,
                    &error,
                    "did not produce a usable file-backed ChatGPT credential",
                )
            })
        }
        CodexCliAction::Logout => match loaded {
            Err(CodexAuthError::Unavailable(_)) => Ok(()),
            Err(error @ (CodexAuthError::Storage(_) | CodexAuthError::Cli(_))) => {
                Err(codex_store_error(
                    action,
                    &error,
                    "could not verify removal of the file-backed credential",
                ))
            }
            Ok(_) => Err(io::Error::other(
                "the official Codex CLI completed logout, but a usable file-backed ChatGPT credential remains",
            )),
        },
    }
}

fn codex_store_error(
    action: CodexCliAction,
    error: &CodexAuthError,
    unavailable_message: &str,
) -> io::Error {
    let operation = codex_operation_name(action);
    let message = match error {
        CodexAuthError::Unavailable(_) => {
            format!("the official Codex CLI completed {operation}, but {unavailable_message}")
        }
        CodexAuthError::Storage(_) => format!(
            "the official Codex CLI completed {operation}, but the Codex credential store is unsafe; CODEX_HOME must be absolute and auth.json must be a private current-user-owned regular non-symlink file on Unix"
        ),
        CodexAuthError::Cli(_) => format!(
            "the official Codex CLI completed {operation}, but Colossus could not validate its file-backed credential state"
        ),
    };
    io::Error::other(message)
}

const fn codex_operation_name(action: CodexCliAction) -> &'static str {
    match action {
        CodexCliAction::Login => "login",
        CodexCliAction::LoginDeviceCode => "device-code login",
        CodexCliAction::Status => "status",
        CodexCliAction::Logout => "logout",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_codex_executable_accepts_platform_spellings() {
        assert!(is_default_codex_executable(Path::new("codex")));
        assert!(is_default_codex_executable(Path::new(
            codex_executable_name()
        )));
        assert!(!is_default_codex_executable(Path::new("/opt/codex")));
    }

    #[cfg(windows)]
    #[test]
    fn windowsapps_codex_alias_is_not_treated_as_runnable_path() {
        assert!(protected_windows_app_package_path(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__2p2nqsd0c76g0\app\resources\codex.exe"
        )));
        assert!(!protected_windows_app_package_path(Path::new(
            r"C:\Users\me\AppData\Local\OpenAI\Codex\bin\123\codex.exe"
        )));
    }
}
