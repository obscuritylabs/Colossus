use std::sync::Arc;

use async_trait::async_trait;
use colossus_contracts::BrowserCapabilities;
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};

/// Trusted composition of one contained native process tree per immutable session.
///
/// No executable, image, endpoint or installation policy originates in an application
/// request. Implementations verify their installation and OS containment independently,
/// and retain native resources before the first launch await. Returned drivers own their
/// proxy, process tree and profile until full teardown; native cancellation is terminal.
/// Launch and cleanup futures must be cancellation safe: deadlines may drop them, but
/// retained supervised resources remain reachable through `reap_failed_launch` or the
/// returned driver's idempotent close. Dropping a future never abandons a process tree.
#[async_trait]
pub trait BrowserHostFactory: Send + Sync {
    /// Immutable accepted installation/containment ceiling, never child self-report.
    fn capabilities(&self) -> BrowserCapabilities;

    /// Launch a dedicated host without allocating its initial native tab yet.
    ///
    /// The request supplies trusted ownership and the whole-process destination envelope.
    /// Partial startup or future cancellation cannot abandon retained cleanup obligations.
    /// A categorical error proves no surviving resources; uncertain cleanup returns unknown.
    async fn launch(
        &self,
        request: &BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError>;

    /// Reap an uncertain partial launch using its retained exact supervised identity.
    ///
    /// Idempotent cleanup never starts a host or renews egress. Only complete proxy/socket
    /// drain, process-tree exit and private profile reclamation permit acknowledgement.
    async fn reap_failed_launch(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError>;
}
