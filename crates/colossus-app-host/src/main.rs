//! Private inherited-channel bridge; the editor owns this process and no public listener.
mod host;
mod wire;

#[tokio::main]
async fn main() {
    if let Err(error) = host::run().await {
        // Raw errors and bootstrap frames can contain private data.
        let code = match error.downcast_ref::<colossus_sdk::SdkError>() {
            Some(colossus_sdk::SdkError::Busy) => "workspace_busy",
            Some(colossus_sdk::SdkError::WorkspaceIdentityChanged) => "workspace_changed",
            Some(colossus_sdk::SdkError::IdentityMismatch) => "bundle_verification_failed",
            Some(colossus_sdk::SdkError::InvalidConfiguration(_)) => "invalid_configuration",
            _ => "managed_start_failed",
        };
        let _ = wire::write(&serde_json::json!({"type":"failed","code":code}));
        std::process::exit(1);
    }
}
