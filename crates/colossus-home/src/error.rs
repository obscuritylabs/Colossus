use std::{io, path::PathBuf};
use thiserror::Error;

/// Failure to resolve or validate owner-private Colossus state.
#[derive(Debug, Error)]
pub enum HomeError {
    /// No current-user home directory is available.
    #[error(
        "cannot resolve the current user's home directory; set COLOSSUS_HOME to an absolute path"
    )]
    HomeDirectoryUnavailable,
    /// The configured Colossus home is not absolute.
    #[error("COLOSSUS_HOME must be an absolute path: {0}")]
    HomeMustBeAbsolute(PathBuf),
    /// A path is linked, has the wrong type, is not owner-private, or changed during use.
    #[error("Colossus private directory is unsafe: {0}")]
    UnsafePrivateDirectory(PathBuf),
    /// A state path escaped its private root or contains an unsafe filesystem object.
    #[error("Colossus confined state path is unsafe: {0}")]
    UnsafeConfinedPath(PathBuf),
    /// The selected workspace cannot supply a stable object identity.
    #[error("cannot establish a stable identity for workspace: {0}")]
    InvalidWorkspace(PathBuf),
    /// A caller supplied an invalid opaque workspace identity.
    #[error("workspace identity is invalid")]
    InvalidWorkspaceIdentity,
    /// An operating-system filesystem operation failed.
    #[error("Colossus home operation failed for {path}: {source}")]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },
}

impl HomeError {
    pub(crate) fn io(path: &std::path::Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}
