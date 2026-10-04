use std::path::PathBuf;

use crate::{DictationError, ModelId, model};

/// A native-selected, verified candidate from the pinned Whisper conversion.
/// The filename is not authority; the bytes must match a reviewed digest.
#[derive(Clone)]
pub struct InstalledModel {
    pub(crate) path: PathBuf,
    pub(crate) digest: &'static str,
    name: &'static str,
    id: ModelId,
}

impl InstalledModel {
    /// Verify one of the two documented English candidates. No model data or
    /// path needs to cross the renderer boundary.
    ///
    /// # Errors
    /// Rejects unknown, missing, linked, or corrupt model assets.
    pub fn open(path: PathBuf) -> Result<Self, DictationError> {
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|_| DictationError::ModelUnavailable)?;
        if !metadata.is_file() {
            return Err(DictationError::ModelUnavailable);
        }
        let id = match metadata.len() {
            77_704_715 => ModelId::TinyEnglish,
            147_964_211 => ModelId::BaseEnglish,
            _ => return Err(DictationError::ModelUnsupported),
        };
        let (name, digest) = (id.name(), id.digest());
        drop(model::verify(&path, digest)?);
        Ok(Self {
            path,
            digest,
            name,
            id,
        })
    }

    /// Bounded, fixed display name; never the selected filesystem path.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Reviewed catalog identity of the verified asset.
    #[must_use]
    pub fn id(&self) -> ModelId {
        self.id
    }
}
