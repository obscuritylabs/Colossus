//! Private macOS browser process-creation broker.
//!
//! The portable codec accepts no executable, environment, descriptor or Mach
//! right nominations. The Darwin owner uses the repository's start-suspended
//! launcher and retains every successful child before independent admission.
//! The explicit Debug Mac factory deploys separate creator and keeper processes;
//! installed production discovery remains disabled. This does not establish an
//! accepted kernel domain or advertise production availability.

mod arguments;
#[cfg(target_os = "macos")]
mod custody_bootstrap;
#[cfg(target_os = "macos")]
mod custody_markers;
#[cfg(unix)]
mod death_fence;
mod owner_death;
mod wire;

mod deployment;
#[cfg(unix)]
mod receipt_output;
#[cfg(unix)]
mod relay;

#[cfg(target_os = "macos")]
mod client;
#[cfg(all(target_os = "macos", feature = "macos-process-entry-development"))]
mod entry;
#[cfg(all(target_os = "macos", feature = "macos-process-output-diagnostic"))]
mod private_output;

#[cfg(target_os = "macos")]
mod creator;
#[cfg(target_os = "macos")]
mod keeper;
#[cfg(target_os = "macos")]
mod owner;

pub use arguments::{HelperRole, SpawnArguments};
#[cfg(all(target_os = "macos", feature = "macos-process-observation-fixture"))]
pub use client::CoreFixtureObservation;
#[cfg(target_os = "macos")]
pub use client::{CoreProcessOwner, DeploymentChannels, DeploymentLaunch};
#[cfg(target_os = "macos")]
pub use creator::{BrowserCreator, CreatorAllocation, CreatorCleanup, MainChannels, MainImage};
#[cfg(unix)]
pub use death_fence::{CreationFence, DeathFence, OwnerLeases};
pub use deployment::{DeploymentImage, DeploymentSetup, ProcessReceipt};
#[cfg(all(target_os = "macos", feature = "macos-process-entry-development"))]
pub use entry::run_native_entry;
#[cfg(target_os = "macos")]
pub use keeper::{KeeperClient, KeeperServer};
#[cfg(target_os = "macos")]
pub use owner::{Allocation, HelperImage, ProcessBroker};
pub use owner_death::OwnerDeath;
pub use wire::{Frame, HEADER_BYTES, MAX_FRAME_BYTES, Message};
