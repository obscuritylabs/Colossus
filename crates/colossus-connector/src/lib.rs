//! Outbound, caller-bound runtime connector. Cloud traffic cannot access worker IPC.
mod connection;
pub use colossus_cloud_executor::ExecutionResources as ConnectorResources;
pub use connection::{ConnectionConfig, ConnectorStatus, RuntimeConnector};
mod enrollment;
pub use enrollment::{EnrollmentStore, EnrollmentTrust};
mod cli;
mod cli_control;
mod cli_prompt;
mod discovery;
mod headless;
mod inventory;
mod localhost;
pub use inventory::native_host_label;
mod outbound;
mod terminal;
pub use cli::{
    ConnectorCommand, EnrollArguments, LocalArguments, PreparedEnrollment, ShareWorkspaceArguments,
    StorageArguments, prepare_enrollment_invitation, run_cli, run_cli_with_invitation,
};
pub use colossus_cloud_protocol::{DeploymentKind, RuntimeInventory, WorkspaceSharing};
pub use headless::{HeadlessCredentialProvider, headless_credential_account};
pub use inventory::native_inventory;
