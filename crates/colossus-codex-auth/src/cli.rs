use crate::CodexAuthError;
use std::{path::Path, process::Stdio};
use tokio::{io::copy, process::Command};

const FILE_CREDENTIAL_STORE_OVERRIDE: &str = "cli_auth_credentials_store=\"file\"";

/// Operator-selected Codex account operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexCliAction {
    /// Start the browser-based ChatGPT sign-in flow.
    Login,
    /// Start the device-code ChatGPT sign-in flow.
    LoginDeviceCode,
    /// Report whether the Codex CLI is signed in.
    Status,
    /// Remove the active Codex CLI sign-in.
    Logout,
}

/// Run an account operation through the official Codex CLI.
///
/// The file credential-store override makes the result reusable by Colossus. OAuth
/// authorization, token validation, and the interactive browser flow remain owned by Codex.
pub async fn run_codex_cli(
    executable: &Path,
    action: CodexCliAction,
) -> Result<(), CodexAuthError> {
    if executable.as_os_str().is_empty() {
        return Err(CodexAuthError::Cli(
            "Codex executable path cannot be empty".into(),
        ));
    }
    let mut command = Command::new(executable);
    command
        .args(codex_cli_arguments(action))
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        CodexAuthError::Cli(format!(
            "failed to run the Codex CLI; install Codex or pass --codex-bin: {error}"
        ))
    })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| CodexAuthError::Cli("failed to capture Codex CLI output".into()))?;
    let forward = tokio::spawn(async move { copy(&mut stdout, &mut tokio::io::stderr()).await });
    let status = child
        .wait()
        .await
        .map_err(|error| CodexAuthError::Cli(format!("failed to wait for Codex CLI: {error}")))?;
    forward
        .await
        .map_err(|error| CodexAuthError::Cli(format!("Codex output task failed: {error}")))?
        .map_err(|error| CodexAuthError::Cli(format!("failed to display Codex output: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CodexAuthError::Cli(format!(
            "Codex CLI account operation exited with {status}"
        )))
    }
}

fn codex_cli_arguments(action: CodexCliAction) -> Vec<&'static str> {
    let mut arguments = vec!["-c", FILE_CREDENTIAL_STORE_OVERRIDE];
    match action {
        CodexCliAction::Login => arguments.push("login"),
        CodexCliAction::LoginDeviceCode => arguments.extend(["login", "--device-auth"]),
        CodexCliAction::Status => arguments.extend(["login", "status"]),
        CodexCliAction::Logout => arguments.push("logout"),
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_commands_force_file_backed_credentials() {
        assert_eq!(
            codex_cli_arguments(CodexCliAction::LoginDeviceCode),
            [
                "-c",
                "cli_auth_credentials_store=\"file\"",
                "login",
                "--device-auth"
            ]
        );
        assert_eq!(
            codex_cli_arguments(CodexCliAction::Status),
            [
                "-c",
                "cli_auth_credentials_store=\"file\"",
                "login",
                "status"
            ]
        );
    }
}
