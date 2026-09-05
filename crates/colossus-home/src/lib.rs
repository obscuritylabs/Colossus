//! Owner-private Colossus home and workspace partition resolution.

mod confined;
mod error;
mod home;
mod identity;
#[cfg(test)]
mod test_support;

pub use confined::{ConfinedFile, ConfinedRoot};
pub use error::HomeError;
pub use home::{ColossusHome, HomeSurface};
#[cfg(target_os = "linux")]
pub use identity::{LinuxWorkspaceIdentity, capture_linux_workspace_identity};
pub use identity::{WorkspaceIdentity, WorkspaceIdentityRef, detect_workspace_identity};
