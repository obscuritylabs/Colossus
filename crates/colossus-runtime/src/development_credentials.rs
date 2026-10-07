use super::*;
use colossus_credentials::{DEVELOPMENT_AUTHORITY_VARIABLE, DevelopmentAuthority};
use colossus_sandbox::ProtectedFilesystem;

/// Check an explicitly selected development authority's process boundary before
/// native hosts acquire runtime state or credentials. This never changes policy.
pub fn validate_development_credential_boundary(
    config: &RuntimeConfig,
) -> Result<(), RuntimeError> {
    validate_selected_boundary(
        config,
        std::env::var_os(DEVELOPMENT_AUTHORITY_VARIABLE).is_some(),
    )
}

fn validate_selected_boundary(config: &RuntimeConfig, selected: bool) -> Result<(), RuntimeError> {
    if !selected {
        return Ok(());
    }
    let supported = match config.sandbox.backend.as_str() {
        "native" => cfg!(any(target_os = "linux", target_os = "macos")),
        "oci" => cfg!(unix),
        "windows_job" => cfg!(windows),
        _ => false,
    };
    if !cfg!(debug_assertions) || !supported || config.sandbox.allow_broker_fallback {
        return Err(RuntimeError::Config(
            "explicit development credential custody requires a supported isolating sandbox without broker fallback".into(),
        ));
    }
    if config.sandbox.environment.iter().any(|name| {
        let name = name.to_ascii_uppercase();
        name.starts_with("COLOSSUS_DEVELOPMENT_")
            || matches!(
                name.as_str(),
                "COLOSSUS_JOURNAL_KEY"
                    | "COLOSSUS_SIGNING_KEY"
                    | "COLOSSUS_DEV_JOURNAL_KEY"
                    | "COLOSSUS_DEV_SIGNING_KEY"
            )
    }) {
        return Err(RuntimeError::Config(
            "development credential control variables cannot be granted to tools".into(),
        ));
    }
    Ok(())
}

pub(super) fn runtime_development_protection(
    config: &RuntimeConfig,
    home: Option<&ConfinedRoot>,
    workspace: &Path,
) -> Result<ProtectedFilesystem, RuntimeError> {
    runtime_development_protection_selected(
        config,
        home,
        std::env::var_os(DEVELOPMENT_AUTHORITY_VARIABLE).is_some(),
        |home| DevelopmentAuthority::selected_root(home, &[workspace.to_owned()]),
    )
}

fn runtime_development_protection_selected(
    config: &RuntimeConfig,
    home: Option<&ConfinedRoot>,
    selected: bool,
    resolve: impl FnOnce(
        &ConfinedRoot,
    ) -> Result<Option<ConfinedRoot>, colossus_contracts::CredentialError>,
) -> Result<ProtectedFilesystem, RuntimeError> {
    validate_selected_boundary(config, selected)?;
    if !selected {
        return Ok(ProtectedFilesystem::default());
    }
    let home = home.ok_or_else(|| {
        RuntimeError::Config(
            "explicit development credential custody requires a retained Colossus home".into(),
        )
    })?;
    let root = resolve(home)
        .map_err(|_| {
            RuntimeError::Config(
                "explicit development credential authority is invalid or inactive".into(),
            )
        })?
        .ok_or_else(|| {
            RuntimeError::Config(
                "development credential selector changed during composition".into(),
            )
        })?;
    ProtectedFilesystem::new(vec![root]).map_err(|_| {
        RuntimeError::Config("development credential authority confinement is invalid".into())
    })
}

#[cfg(test)]
mod tests;
