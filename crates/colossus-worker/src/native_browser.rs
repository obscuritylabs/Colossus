//! Separately authenticated native GUI authority. Ordinary worker clients cannot use this endpoint.
use super::*;
use colossus_browser_presentation::native_admission::{
    NativeBrowserEnrollment, NativeBrowserReply, NativeBrowserRequest, server_handshake,
};
use colossus_grpc::{CredentialRepository as _, JournalCredentialRepository};
use colossus_runtime::{RuntimeNativeBrowserAuthority, RuntimeNativeBrowserSession};
use std::sync::atomic::AtomicBool;
use zeroize::Zeroizing;

mod lifecycle;
mod platform;
mod relay;
mod serve;
#[cfg(all(test, unix))]
mod tests;

const MAX_NATIVE_CONNECTIONS: usize = 8;
const MAX_LIFETIME_ADMISSIONS: usize = 256;

/// Compiler/native composition inputs; this type has no renderer/model serialization.
pub struct NativeBrowserServerConfig {
    instance: PathBuf,
    enrollment: NativeBrowserEnrollment,
    authentication: Zeroizing<[u8; 32]>,
    primary_credential_id: String,
    credentials: Arc<PublicApiCredentialManager>,
}
impl NativeBrowserServerConfig {
    /// Bind independent native authority to this verified child's private instance and primary grant.
    /// The caller must obtain every input directly from the inherited attested bootstrap.
    pub fn new(
        instance: impl Into<PathBuf>,
        enrollment: NativeBrowserEnrollment,
        authentication: Zeroizing<[u8; 32]>,
        primary_credential_id: String,
        credentials: Arc<PublicApiCredentialManager>,
    ) -> Result<Self, WorkerError> {
        enrollment
            .validate()
            .map_err(|_| WorkerError::Protocol("native browser enrollment is invalid".into()))?;
        let instance = instance.into();
        if !instance.is_absolute()
            || *authentication == [0; 32]
            || Uuid::parse_str(&primary_credential_id).is_err()
        {
            return Err(WorkerError::Protocol(
                "native browser authority is invalid".into(),
            ));
        }
        platform::validate_parent(enrollment.parent_process_id)?;
        Ok(Self {
            instance,
            enrollment,
            authentication,
            primary_credential_id,
            credentials,
        })
    }
}

pub(super) struct PreparedNativeBrowser {
    listener: platform::Listener,
    service: Arc<NativeBrowserService>,
    slots: Arc<tokio::sync::Semaphore>,
}
pub(super) struct NativeBrowserService {
    runtime: Arc<Runtime>,
    authority: Option<Arc<RuntimeNativeBrowserAuthority>>,
    repository: JournalCredentialRepository,
    primary_credential_id: String,
    enrollment: NativeBrowserEnrollment,
    authentication: Zeroizing<[u8; 32]>,
    stopped: AtomicBool,
    sessions:
        Mutex<BTreeMap<colossus_contracts::BrowserSessionId, Arc<RuntimeNativeBrowserSession>>>,
    retired: Mutex<BTreeSet<colossus_contracts::BrowserSessionId>>,
    detached: Mutex<BTreeSet<colossus_contracts::BrowserSessionId>>,
    cleanup: tokio::sync::Mutex<()>,
    admitted: std::sync::atomic::AtomicUsize,
}
impl WorkerServer {
    /// Prepare a distinct native-browser endpoint before the inherited activation exchange.
    /// A missing accepted Embedded host remains available only as a categorical negative probe.
    pub async fn with_native_browser_api(
        mut self,
        config: NativeBrowserServerConfig,
    ) -> Result<Self, WorkerError> {
        if self.native_browser.is_some()
            || !Arc::ptr_eq(&self.runtime.journal(), &config.credentials.journal())
            || self.authentication_key.expose() == config.authentication.as_ref()
        {
            return Err(WorkerError::Protocol(
                "native browser composition is invalid".into(),
            ));
        }
        let repository = JournalCredentialRepository::new(config.credentials.journal());
        if !repository
            .get(&config.primary_credential_id)
            .is_ok_and(|record| {
                record.is_some_and(|record| {
                    !record.is_revoked()
                        && record.application_id() == config.enrollment.application_id
                })
            })
        {
            return Err(WorkerError::Protocol(
                "native primary grant is not owned".into(),
            ));
        }
        let authority = match self.runtime.bind_native_browser_authority(
            &config.enrollment.application_id,
            Uuid::from_bytes(config.enrollment.instance),
        ) {
            Ok(authority) => Some(Arc::new(authority)),
            Err(colossus_runtime::RuntimeNativeBrowserError::Unavailable) => None,
            Err(_) => {
                return Err(WorkerError::Protocol(
                    "native browser authority rejected".into(),
                ));
            }
        };
        let listener = platform::Listener::bind(&config.instance, &config.enrollment).await?;
        self.native_browser = Some(PreparedNativeBrowser {
            listener,
            slots: Arc::new(tokio::sync::Semaphore::new(MAX_NATIVE_CONNECTIONS)),
            service: Arc::new(NativeBrowserService {
                runtime: Arc::clone(&self.runtime),
                authority,
                repository,
                primary_credential_id: config.primary_credential_id,
                enrollment: config.enrollment,
                authentication: config.authentication,
                stopped: AtomicBool::new(false),
                sessions: Mutex::new(BTreeMap::new()),
                retired: Mutex::new(BTreeSet::new()),
                detached: Mutex::new(BTreeSet::new()),
                cleanup: tokio::sync::Mutex::new(()),
                admitted: std::sync::atomic::AtomicUsize::new(0),
            }),
        });
        Ok(self)
    }
}
impl NativeBrowserService {
    fn authorized(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self
                .repository
                .get(&self.primary_credential_id)
                .is_ok_and(|record| {
                    record.is_some_and(|record| {
                        record.is_active()
                            && record.application_id() == self.enrollment.application_id
                    })
                })
    }
    fn available(&self) -> bool {
        self.authorized()
            && self
                .authority
                .as_ref()
                .is_some_and(|authority| self.runtime.native_browser_available(authority))
    }
    fn tracked(
        &self,
        session: &colossus_contracts::BrowserSessionId,
    ) -> Option<Arc<RuntimeNativeBrowserSession>> {
        self.sessions.lock().ok()?.get(session).cloned()
    }
}
