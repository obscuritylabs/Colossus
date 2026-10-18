//! Explicit unsupported-platform adapter. It cannot start a native helper.
use crate::TrustedComputerContext;
use async_trait::async_trait;
use colossus_contracts::{ComputerBinding, ComputerCapabilities, ComputerLimits};
use colossus_ports::{
    ComputerDriver, ComputerDriverError as Error, ComputerDriverReply, ComputerDriverRequest,
    RunControl,
};

/// Native computer use is available only in verified Windows host composition.
pub struct NativeComputerDriver;
impl NativeComputerDriver {
    /// Other platforms cannot construct a desktop-control adapter.
    pub fn new(_: std::path::PathBuf, _: TrustedComputerContext) -> Result<Self, Error> {
        Err(Error::Unavailable)
    }
}
#[async_trait]
impl ComputerDriver for NativeComputerDriver {
    fn capabilities(&self) -> ComputerCapabilities {
        ComputerCapabilities {
            available: false,
            platform: std::env::consts::OS.into(),
            limits: ComputerLimits::HARD_MAX,
            unavailable_reason: Some("unsupported_platform".into()),
        }
    }
    async fn execute(
        &self,
        _: ComputerDriverRequest,
        _: &RunControl,
    ) -> Result<ComputerDriverReply, Error> {
        Err(Error::Unavailable)
    }
    fn revoke_run(&self, _: &ComputerBinding) {}
    async fn close_run(&self, _: &ComputerBinding) -> Result<(), Error> {
        Ok(())
    }
}
