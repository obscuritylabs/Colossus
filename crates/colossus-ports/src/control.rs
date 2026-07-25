use super::*;

/// Cloneable cooperative cancellation signal shared by interfaces and the agent loop.
#[derive(Clone, Default)]
pub struct RunControl {
    cancelled: Arc<AtomicBool>,
}

impl RunControl {
    /// Request cancellation at the next safe application boundary.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Return whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Journal and repository failure.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The expected stream version did not match the durable version.
    #[error(
        "optimistic concurrency conflict for {stream_id}: expected {expected}, actual {actual}"
    )]
    Conflict {
        /// Aggregate stream identifier.
        stream_id: String,
        /// Caller expectation.
        expected: u64,
        /// Durable version.
        actual: u64,
    },
    /// Journal verification failed and the runtime must recover read-only.
    #[error("journal verification failed: {0}")]
    Verification(String),
    /// Requested record does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The configured key is absent or invalid.
    #[error("key unavailable: {0}")]
    KeyUnavailable(String),
    /// Another live runtime owns the canonical single-writer lease.
    #[error("runtime writer lease is already held")]
    WriterLeaseHeld,
    /// The configured workspace pathname no longer names the directory whose
    /// identity was captured by the trusted host or running runtime.
    #[error("workspace identity changed")]
    WorkspaceIdentityChanged,
    /// Adapter-specific failure with secrets removed.
    #[error("storage adapter failure: {0}")]
    Adapter(String),
    /// An external mutation may have occurred and must not be retried automatically.
    #[error("external storage outcome is unknown: {0}")]
    OutcomeUnknown(String),
    /// Writes are disabled because startup verification failed.
    #[error("runtime is in read-only recovery mode")]
    RecoveryMode,
}

/// Provider-turn failure classification preserved across the application port.
#[derive(Debug, Error)]
pub enum ModelProviderError {
    /// Profile or request configuration is invalid.
    #[error("provider configuration failed: {0}")]
    Configuration(String),
    /// A bounded correction turn may safely be attempted.
    #[error("recoverable provider failure {code}: {message}")]
    Recoverable {
        /// Stable machine-readable recovery code.
        code: String,
        /// Bounded safe diagnostic.
        message: String,
        /// HTTP response status when the failure came from a provider response.
        http_status: Option<u16>,
        /// Bounded provider retry lower bound when supplied.
        retry_after_ms: Option<u64>,
    },
    /// Provider returned a known non-success HTTP response.
    #[error("provider turn failed: {message}")]
    HttpStatus {
        /// HTTP response status.
        status: u16,
        /// Bounded safe diagnostic without response headers or body.
        message: String,
    },
    /// Provider failed with a known terminal outcome.
    #[error("provider turn failed: {0}")]
    Failed(String),
    /// The external outcome cannot be proven and must not be retried.
    #[error("provider outcome is unknown: {0}")]
    OutcomeUnknown(String),
}

/// Search routing, authorization, transport, or normalization failure.
#[derive(Debug, Error)]
pub enum SearchError {
    /// Search is not configured for the requested logical role.
    #[error("search route unavailable: {0}")]
    Unavailable(String),
    /// Search profile or request configuration is invalid.
    #[error("search configuration failed: {0}")]
    Configuration(String),
    /// Policy or approval denied the search before release.
    #[error("search denied: {0}")]
    Denied(String),
    /// Search failed with a known terminal outcome.
    #[error("search failed: {0}")]
    Failed(String),
    /// A dispatched external search may have consumed provider resources.
    #[error("search outcome is unknown: {0}")]
    OutcomeUnknown(String),
}

/// Tool lookup, validation, policy, or execution failure.
#[derive(Debug, Error)]
pub enum ToolError {
    /// Tool name is absent from the active catalog.
    #[error("unknown tool: {0}")]
    Unknown(String),
    /// Arguments failed the strict registered schema.
    #[error("invalid arguments for {tool}: {message}")]
    InvalidArguments {
        /// Requested tool.
        tool: String,
        /// Bounded validation detail.
        message: String,
    },
    /// Policy or approval denied the effect before execution.
    #[error("tool execution denied: {0}")]
    Denied(String),
    /// Adapter reported a known failure.
    #[error("tool execution failed: {0}")]
    Failed(String),
    /// Tool effect may have occurred and cannot be retried implicitly.
    #[error("tool outcome is unknown: {0}")]
    OutcomeUnknown(String),
}

/// Context preparation or snapshot lifecycle failure.
#[derive(Debug, Error)]
pub enum ContextError {
    /// Context configuration or request cannot satisfy the safety contract.
    #[error("context configuration failed: {0}")]
    Configuration(String),
    /// Canonical snapshot persistence or session reconstruction failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Optional model summarization failed before deterministic fallback could run.
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
}

/// Result of full journal verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerificationReport {
    /// Number of verified records.
    pub event_count: u64,
    /// Highest verified global sequence.
    pub last_sequence: u64,
    /// Hash at the verified chain head.
    pub last_hash: String,
    /// Latest verified checkpoint, if present.
    pub checkpoint: Option<SignedCheckpoint>,
}
