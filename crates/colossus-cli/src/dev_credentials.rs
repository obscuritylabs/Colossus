//! Offline CLI translation for explicitly reviewed development credential custody.

use super::{
    dev_credentials_apply,
    dev_credentials_args::{DevelopmentCredentialsAction, DevelopmentCredentialsCommand},
    dev_credentials_plan::{self as plan, Failure, Result},
    dev_credentials_sources,
    public_api_admin::OsCredentialStore,
};
use colossus_credentials::DevelopmentAuthority;
use serde_json::json;
use std::path::Path;

pub(super) fn run(command: &DevelopmentCredentialsCommand, workspace: &Path) -> Result<()> {
    // Reject release builds before creating directories, opening a private key,
    // inspecting an OS entry, or acquiring any runtime/configuration authority.
    if !cfg!(debug_assertions) {
        return Err(Failure(
            "development credential custody is unavailable in release builds",
        ));
    }
    match &command.command {
        DevelopmentCredentialsAction::Init { home, workspace } => {
            plan::clean_absolute(home)?;
            let workspace = std::fs::canonicalize(workspace)
                .map_err(|_| Failure("workspace constraint is unavailable"))?;
            if !workspace.is_dir() {
                return Err(Failure("workspace constraint must be a directory"));
            }
            let home = colossus_home::ColossusHome::ensure_at(home)
                .map_err(|_| Failure("development home is unsafe"))?;
            let _authority = DevelopmentAuthority::initialize(home.confined_root(), std::slice::from_ref(&workspace)).map_err(|_| Failure("development authority could not be prepared; existing or partial custody was not replaced"))?;
            status(home.confined_root(), &workspace, false)?;
        }
        DevelopmentCredentialsAction::Status { home, workspace } => {
            let home = plan::existing_root(home)?;
            let workspace = std::fs::canonicalize(workspace)
                .map_err(|_| Failure("workspace constraint is unavailable"))?;
            status(&home, &workspace, false)?;
        }
        DevelopmentCredentialsAction::Plan { sources, plan_file } => {
            let selected = dev_credentials_sources::make_plan(sources, workspace)?;
            if selected.fresh_empty
                && plan_file.parent()
                    != Some(
                        DevelopmentAuthority::path_for_home(&plan::existing_root(&selected.home)?)
                            .as_path(),
                    )
            {
                return Err(Failure(
                    "fresh-home plans must be written directly inside the prepared authority directory",
                ));
            }
            let digest = plan::write_plan(plan_file, &selected)?;
            super::print_json(&json!({ "action": "plan", "planFile": plan_file, "planSha256": digest, "authorityPath": selected.home.join("development-credentials"), "sources": selected.sources, "metadataOnly": true, "sourceUnchanged": true })).map_err(|_| Failure("development plan report could not be written"))?;
        }
        DevelopmentCredentialsAction::Rewrap {
            plan_file,
            expected_plan_sha256,
            apply,
        } => {
            if !apply {
                return Err(Failure(
                    "rewrap requires explicit --apply and the reviewed plan digest",
                ));
            }
            let selected = plan::read_plan(plan_file, expected_plan_sha256)?;
            let (copied, reused) = dev_credentials_apply::apply(&selected, &OsCredentialStore)?;
            super::print_json(&json!({ "action": "rewrap", "authorityPath": selected.home.join("development-credentials"), "active": true, "copied": copied, "reused": reused, "sourceEntriesRetained": true, "credentialsReissued": false, "enrollmentsChanged": false })).map_err(|_| Failure("development custody report could not be written"))?;
        }
    }
    Ok(())
}

fn status(
    home: &colossus_home::ConfinedRoot,
    workspace: &Path,
    activated_by_this_command: bool,
) -> Result<()> {
    let path = DevelopmentAuthority::path_for_home(home);
    let metadata = DevelopmentAuthority::metadata(home, &path, &[workspace.to_owned()])
        .map_err(|_| Failure("development authority marker is unavailable or invalid"))?;
    super::print_json(&json!({ "authorityPath": path, "active": metadata.active, "activatedByThisCommand": activated_by_this_command, "sourceUnchanged": true })).map_err(|_| Failure("development authority status could not be written"))
}
