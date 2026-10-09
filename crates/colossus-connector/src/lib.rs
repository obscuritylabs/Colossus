//! Outbound, caller-bound runtime connector. Cloud traffic cannot access worker IPC.
mod connection;
mod resources;
pub use connection::{ConnectionConfig, ConnectorStatus, RuntimeConnector};
pub use resources::ConnectorResources;
mod enrollment;
pub use enrollment::EnrollmentStore;
mod cli;
mod cli_control;
mod discovery;
mod headless;
mod history;
mod inventory;
pub use inventory::native_host_label;
mod outbound;
mod released;
pub use cli::{
    ConnectorCommand, EnrollArguments, LocalArguments, ShareWorkspaceArguments, StorageArguments,
    run_cli,
};
pub use colossus_cloud_protocol::{DeploymentKind, RuntimeInventory, WorkspaceSharing};
pub use headless::{HeadlessCredentialProvider, headless_credential_account};
pub use inventory::native_inventory;
