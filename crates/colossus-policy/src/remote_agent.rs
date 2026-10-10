//! Future outgoing agent adapter boundary. No remote peer is enabled by this port.

use crate::EffectExecutor;

/// Outgoing remote-agent adapter behind the ordinary one-use effect permit.
///
/// Implementations execute only runtime-normalized requests naming an operator-bound
/// peer. The inherited [`EffectExecutor`] contract requires an authenticated,
/// non-cloneable permit and quarantines the response until post-effect release.
/// Discovery, submission, polling, cancellation, and artifact retrieval must use
/// separate policy actions. This port grants no URL, credential, or execution authority.
pub trait RemoteAgentClient: EffectExecutor {}
