//! Independent consent storage: ordinary settings snapshots cannot restore revoked grants.
use super::*;
use crate::remembered_approvals::{CommandAllowance, validate_saved};

const FILE: &str = "remembered-commands.json";
const MAX_BYTES: u64 = 128 * 1024;

impl SettingsStore {
    pub(crate) fn command_allowances(&self) -> Result<Vec<CommandAllowance>, CommandErrorDto> {
        let path = self.root.join(FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(storage_error()),
            Ok(_) => {}
        }
        let rules: Vec<CommandAllowance> =
            serde_json::from_slice(&read_private_file(&path, MAX_BYTES)?)
                .map_err(|_| storage_error())?;
        validate_saved(&rules)?;
        Ok(rules)
    }

    pub(crate) fn save_command_allowances(
        &self,
        rules: &[CommandAllowance],
    ) -> Result<(), CommandErrorDto> {
        validate_saved(rules)?;
        let bytes = serde_json::to_vec(rules).map_err(|_| storage_error())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(storage_error());
        }
        let temporary = self.root.join(format!(".{FILE}.{}.tmp", Uuid::new_v4()));
        write_private_file(&temporary, &bytes)?;
        let result = (|| {
            replace_private_file(&temporary, &self.root.join(FILE))?;
            sync_private_directory(&self.root)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}
