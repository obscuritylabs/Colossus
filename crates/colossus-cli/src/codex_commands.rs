use colossus_codex_auth::{CodexAuthError, CodexAuthStore, CodexCliAction, run_codex_cli};
use std::{error::Error, io, path::Path};

pub(super) async fn run_codex_account_operation(
    executable: &Path,
    action: CodexCliAction,
) -> Result<(), Box<dyn Error>> {
    let store = CodexAuthStore::from_environment().map_err(codex_store_resolution_error)?;
    verify_codex_account_precondition(&store, action)?;
    run_codex_cli(executable, action).await?;
    verify_codex_account_state(&store, action)?;
    Ok(())
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
