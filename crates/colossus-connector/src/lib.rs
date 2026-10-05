//! Outbound, caller-bound runtime connector. Cloud traffic cannot access worker IPC.
mod connection;
pub use connection::{ConnectionConfig, ConnectorStatus, RuntimeConnector};
mod enrollment;
pub use enrollment::EnrollmentStore;
mod cli;
mod cli_control;
mod headless;
mod released;
pub use cli::{ConnectorCommand, EnrollArguments, LocalArguments, StorageArguments, run_cli};
pub use headless::{HeadlessCredentialProvider, headless_credential_account};
