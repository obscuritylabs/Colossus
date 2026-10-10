use super::{BrowserProfileEngine, BrowserProfileError, files};
use colossus_contracts::{BrowserProfileId, BrowserProfileSummary};
use colossus_home::{ConfinedFile, ConfinedRoot};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read as _, Write as _},
    path::Path,
};

const MAX_METADATA: u64 = 4096;

#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    Clean,
    Active,
    Dirty,
    Resetting,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub(super) schema_version: u16,
    pub(super) id: BrowserProfileId,
    pub(super) owner: String,
    pub(super) name: String,
    pub(super) engine: BrowserProfileEngine,
    pub(super) state: State,
}
impl Manifest {
    pub(super) fn summary(&self) -> BrowserProfileSummary {
        BrowserProfileSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            reset_required: self.state == State::Dirty,
        }
    }
}
fn encode(manifest: &Manifest) -> Result<Vec<u8>, BrowserProfileError> {
    let bytes = serde_json::to_vec(manifest).map_err(|_| BrowserProfileError::Denied)?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(BrowserProfileError::LimitExceeded);
    }
    Ok(bytes)
}
pub(super) fn create(root: &ConfinedRoot, manifest: &Manifest) -> Result<(), BrowserProfileError> {
    let file = root
        .open_file(Path::new("profile.json"))
        .map_err(|_| BrowserProfileError::Denied)?;
    if !file.was_created() {
        return Err(BrowserProfileError::Denied);
    }
    file.file()
        .write_all(&encode(manifest)?)
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    file.file()
        .sync_all()
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    file.revalidate(root)
        .map_err(|_| BrowserProfileError::OutcomeUnknown)
}
pub(super) fn read(
    root: &ConfinedRoot,
    owner: &str,
    id: &BrowserProfileId,
) -> Result<(ConfinedFile, Manifest), BrowserProfileError> {
    let file = root
        .open_existing_file(Path::new("profile.json"))
        .map_err(|_| BrowserProfileError::Denied)?;
    if file
        .file()
        .metadata()
        .map_err(|_| BrowserProfileError::Denied)?
        .len()
        > MAX_METADATA
    {
        return Err(BrowserProfileError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    file.file()
        .take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BrowserProfileError::Denied)?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(BrowserProfileError::LimitExceeded);
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| BrowserProfileError::Denied)?;
    if manifest.schema_version != 1
        || manifest.owner != owner
        || &manifest.id != id
        || manifest.name.is_empty()
        || manifest.name.len() > 80
        || manifest.name.chars().any(char::is_control)
    {
        return Err(BrowserProfileError::Denied);
    }
    manifest.engine.validate()?;
    file.revalidate(root)
        .map_err(|_| BrowserProfileError::Denied)?;
    Ok((file, manifest))
}
pub(super) fn replace(
    root: &ConfinedRoot,
    existing: &mut ConfinedFile,
    manifest: &Manifest,
) -> Result<(), BrowserProfileError> {
    let name = format!("metadata-{}.json", files::nonce()?);
    let staged = root
        .open_file(Path::new(&name))
        .map_err(|_| BrowserProfileError::Denied)?;
    if !staged.was_created() {
        return Err(BrowserProfileError::Denied);
    }
    staged
        .file()
        .write_all(&encode(manifest)?)
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    *existing = root
        .replace_existing_file(&staged, existing)
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    Ok(())
}
