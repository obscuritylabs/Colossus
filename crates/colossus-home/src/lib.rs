//! Owner-private Colossus home and workspace partition resolution.

mod confined;
mod error;
mod home;
mod identity;

pub use confined::{ConfinedFile, ConfinedRoot};
pub use error::HomeError;
pub use home::{ColossusHome, HomeSurface};
pub use identity::{WorkspaceIdentity, WorkspaceIdentityRef, detect_workspace_identity};
