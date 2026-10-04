use crate::{DictationError, InstalledModel, ModelId};
use colossus_home::{ColossusHome, ConfinedRoot};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_SETTINGS_BYTES: u64 = 8192;
static NEXT_WRITE: AtomicU64 = AtomicU64::new(1);

/// Device-local preferences shared by Desktop and the local terminal interface.
/// Model paths remain native and must never be serialized into renderer DTOs.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DictationPreferences {
    /// Version of this native preference document.
    pub schema_version: u32,
    /// User has enabled microphone dictation; recording still requires an explicit start.
    pub enabled: bool,
    /// Selected reviewed model.
    pub model: ModelId,
    /// Native-only path of an explicitly selected or installed asset.
    pub model_path: Option<PathBuf>,
    /// Opaque identity of the preferred local input, or the OS default.
    pub microphone: Option<String>,
    /// Interpret spoken punctuation commands in the draft.
    pub spoken_punctuation: bool,
}

impl Default for DictationPreferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: false,
            model: ModelId::default(),
            model_path: None,
            microphone: None,
            spoken_punctuation: true,
        }
    }
}

/// Owner-private native settings and model storage beneath the Colossus home.
pub struct DictationSettings {
    root: ConfinedRoot,
}

impl DictationSettings {
    /// Open the shared device-local dictation directory, honoring `COLOSSUS_HOME`.
    /// # Errors
    /// Rejects linked, non-private, or unavailable settings directories.
    pub fn discover() -> Result<Self, DictationError> {
        let home = ColossusHome::resolve_and_ensure().map_err(|_| DictationError::Settings)?;
        let directory = home
            .confined_root()
            .prepare_directory(Path::new("dictation"))
            .map_err(|_| DictationError::Settings)?;
        Self::at(directory)
    }
    /// Open an explicit owner-private directory, principally for isolated callers.
    /// # Errors
    /// Rejects unsafe filesystem authority.
    pub fn at(path: PathBuf) -> Result<Self, DictationError> {
        Ok(Self {
            root: ConfinedRoot::bind(path).map_err(|_| DictationError::Settings)?,
        })
    }
    /// Read a bounded preference document; missing preferences use safe defaults.
    /// # Errors
    /// Rejects unknown schemas, malformed documents, links, and oversized input.
    pub fn load(&self) -> Result<DictationPreferences, DictationError> {
        let path = self
            .root
            .prepare_file(Path::new("settings.json"))
            .map_err(|_| DictationError::Settings)?;
        if !path.exists() {
            return Ok(DictationPreferences::default());
        }
        let file = self
            .root
            .open_existing_file(Path::new("settings.json"))
            .map_err(|_| DictationError::Settings)?;
        let mut bytes = Vec::new();
        file.file()
            .take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| DictationError::Settings)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(DictationError::Settings);
        }
        let preferences: DictationPreferences =
            serde_json::from_slice(&bytes).map_err(|_| DictationError::Settings)?;
        validate(&preferences)?;
        Ok(preferences)
    }
    /// Atomically save one validated preference document without exposing native paths.
    /// # Errors
    /// Rejects unsafe or invalid settings and reports persistence failures.
    pub fn save(&self, preferences: &DictationPreferences) -> Result<(), DictationError> {
        validate(preferences)?;
        let bytes = serde_json::to_vec(preferences).map_err(|_| DictationError::Settings)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(DictationError::Settings);
        }
        let destination = self
            .root
            .prepare_file(Path::new("settings.json"))
            .map_err(|_| DictationError::Settings)?;
        let temporary = format!(
            "settings-{}-{}.tmp",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        );
        let file = self
            .root
            .open_file(Path::new(&temporary))
            .map_err(|_| DictationError::Settings)?;
        let result = (|| {
            file.file()
                .set_len(0)
                .map_err(|_| DictationError::Settings)?;
            let mut writer = file.file();
            writer
                .write_all(&bytes)
                .and_then(|()| writer.sync_all())
                .map_err(|_| DictationError::Settings)?;
            file.revalidate(&self.root)
                .map_err(|_| DictationError::Settings)?;
            std::fs::rename(file.path(), destination).map_err(|_| DictationError::Settings)?;
            self.root
                .sync_directory()
                .map_err(|_| DictationError::Settings)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(file.path());
        }
        result
    }
    /// Confined path for a reviewed optional model.
    /// # Errors
    /// Rejects unsafe model-cache paths.
    pub fn model_path(&self, model: ModelId) -> Result<PathBuf, DictationError> {
        self.root
            .prepare_file(&Path::new("models").join(model.filename()))
            .map_err(|_| DictationError::Settings)
    }
    /// Resolve a saved model and verify its actual bytes before recording.
    /// # Errors
    /// Rejects absent, corrupt, unknown, or mismatched assets.
    pub fn selected_model(
        &self,
        preferences: &DictationPreferences,
    ) -> Result<InstalledModel, DictationError> {
        let path = match &preferences.model_path {
            Some(path) => path.clone(),
            None => self.model_path(preferences.model)?,
        };
        let model = InstalledModel::open(path)?;
        if model.id() != preferences.model {
            return Err(DictationError::ModelIntegrity);
        }
        Ok(model)
    }
    /// Install one reviewed asset into the shared private cache.
    /// # Errors
    /// Rejects unreviewed bytes and filesystem failures; existing valid assets are reused.
    pub fn install(&self, source: &Path) -> Result<ModelId, DictationError> {
        let model = InstalledModel::open(source.to_owned())?;
        let destination = self.model_path(model.id())?;
        if InstalledModel::open(destination.clone())
            .is_ok_and(|existing| existing.id() == model.id())
        {
            return Ok(model.id());
        }
        let temporary = destination.with_extension(format!(
            "{}-{}.part",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let relative = self
            .root
            .relative(&temporary)
            .map_err(|_| DictationError::Settings)?;
        let output = self
            .root
            .open_file(relative)
            .map_err(|_| DictationError::Settings)?;
        let result = (|| {
            output
                .file()
                .set_len(0)
                .map_err(|_| DictationError::Settings)?;
            let mut input =
                std::fs::File::open(source).map_err(|_| DictationError::ModelUnavailable)?;
            let mut writer = output.file();
            std::io::copy(
                &mut std::io::Read::by_ref(&mut input).take(model.id().bytes() + 1),
                &mut writer,
            )
            .map_err(|_| DictationError::Settings)?;
            writer.sync_all().map_err(|_| DictationError::Settings)?;
            InstalledModel::open(temporary.clone())?;
            output
                .revalidate(&self.root)
                .map_err(|_| DictationError::Settings)?;
            std::fs::rename(&temporary, &destination).map_err(|_| DictationError::Settings)?;
            self.root
                .sync_directory()
                .map_err(|_| DictationError::Settings)?;
            Ok(model.id())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}

fn validate(preferences: &DictationPreferences) -> Result<(), DictationError> {
    if preferences.schema_version != 1
        || preferences
            .model_path
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        || preferences
            .microphone
            .as_ref()
            .is_some_and(|id| id.len() != 64 || !id.bytes().all(|value| value.is_ascii_hexdigit()))
    {
        return Err(DictationError::Settings);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
