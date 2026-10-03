use std::path::PathBuf;

use crate::{DictationError, model};

/// A native-selected, verified candidate from the pinned Whisper conversion.
/// The filename is not authority; the bytes must match a reviewed digest.
#[derive(Clone)]
pub struct InstalledModel {
    pub(crate) path: PathBuf,
    pub(crate) digest: &'static str,
    name: &'static str,
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
        let (name, digest) = match metadata.len() {
            77_704_715 => (
                "Tiny English",
                "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
            ),
            147_964_211 => (
                "Base English",
                "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
            ),
            _ => return Err(DictationError::ModelUnsupported),
        };
        drop(model::verify(&path, digest)?);
        Ok(Self { path, digest, name })
    }

    /// Bounded, fixed display name; never the selected filesystem path.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }
}
