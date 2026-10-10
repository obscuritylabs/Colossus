use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use colossus_contracts::{
    BrowserCapabilities, BrowserObservation, BrowserSessionId, BrowserTabSummary,
};
use colossus_ports::{
    BrowserCancelDisposition, BrowserDownloadDescriptor, BrowserDownloadReadRequest, BrowserDriver,
    BrowserDriverCommand, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
    BrowserNativeHandoffRequest, BrowserScreenshotChunk, BrowserScreenshotDescriptor,
    BrowserScreenshotReadRequest, BrowserUploadCommitRequest, BrowserUploadPrepareRequest,
    BrowserUploadReceipt, BrowserUploadWriteRequest, RunControl,
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, oneshot, watch};

use crate::BrowserHostFactory;

mod capture;
mod cleanup;
mod operations;
#[cfg(test)]
mod tests;
mod transfer;
mod wait;

type CleanupResult = Option<Result<(), BrowserDriverError>>;

#[derive(Clone, Copy)]
enum LaunchState {
    Pending,
    Absent,
    Unknown,
    Ready,
}

struct State {
    driver: Option<Arc<dyn BrowserDriver>>,
    launch: LaunchState,
    terminal: bool,
    cleaned: bool,
    cleanup: Option<watch::Receiver<CleanupResult>>,
    upload: Option<transfer::Upload>,
    download: Option<transfer::Download>,
}

struct Entry {
    request: BrowserDriverOpenRequest,
    authority: RunControl,
    admission_fenced: AtomicBool,
    state: Mutex<State>,
    operation: AsyncMutex<()>,
}

struct Inner {
    factory: Arc<dyn BrowserHostFactory>,
    capabilities: BrowserCapabilities,
    entries: Mutex<BTreeMap<BrowserSessionId, Arc<Entry>>>,
    actions: Arc<Semaphore>,
}

/// Dedicated native host ownership behind the runtime's existing browser driver port.
///
/// A session is retained before launch. Allocation/actions have independent bounded
/// owners, so dropping a caller revokes authority and starts cleanup without abandoning
/// a late native result. Cancellation terminally tears down the dedicated host; a new
/// page requires a new session and document. Unknown teardown retains its obligation.
pub struct BrowserHostPool {
    inner: Arc<Inner>,
}

impl BrowserHostPool {
    /// Install only a trusted factory whose accepted ceiling is independently verified.
    /// An unavailable factory remains discoverably unavailable and performs no launch.
    pub fn new(factory: Arc<dyn BrowserHostFactory>) -> Result<Self, BrowserDriverError> {
        let capabilities = factory.capabilities();
        if capabilities.available {
            crate::validation::capabilities(&capabilities)?;
        }
        let actions = Arc::new(Semaphore::new(usize::from(
            capabilities.limits.max_concurrent_actions,
        )));
        Ok(Self {
            inner: Arc::new(Inner {
                factory,
                capabilities,
                entries: Mutex::new(BTreeMap::new()),
                actions,
            }),
        })
    }

    /// Native obligations still awaiting full proxy/process/profile teardown.
    pub fn owned_session_count(&self) -> Result<usize, BrowserDriverError> {
        let entries = self
            .inner
            .entries
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let mut count = 0;
        for entry in entries.values() {
            if !entry
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
                .cleaned
            {
                count += 1;
            }
        }
        Ok(count)
    }

    fn entry(&self, id: &BrowserSessionId) -> Result<Option<Arc<Entry>>, BrowserDriverError> {
        Ok(self
            .inner
            .entries
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .get(id)
            .cloned())
    }
}

#[async_trait]
impl BrowserDriver for BrowserHostPool {
    fn cancellation_disposition(&self) -> BrowserCancelDisposition {
        BrowserCancelDisposition::ClosesContext
    }

    fn capabilities(&self) -> BrowserCapabilities {
        if self.inner.capabilities.available && self.inner.factory.capabilities().available {
            self.inner.capabilities.clone()
        } else {
            BrowserCapabilities::unavailable()
        }
    }

    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if !self.capabilities().available {
            return Err(BrowserDriverError::Unavailable);
        }
        crate::validation::allocation(&request, &self.inner.capabilities)?;
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        tokio::runtime::Handle::try_current().map_err(|_| BrowserDriverError::Unavailable)?;
        let entry = Arc::new(Entry {
            request,
            authority: RunControl::default(),
            admission_fenced: AtomicBool::new(false),
            state: Mutex::new(State {
                driver: None,
                launch: LaunchState::Pending,
                terminal: false,
                cleaned: false,
                cleanup: None,
                upload: None,
                download: None,
            }),
            operation: AsyncMutex::new(()),
        });
        {
            let mut entries = self
                .inner
                .entries
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if entries.contains_key(&entry.request.session_id) {
                return Err(BrowserDriverError::Stale);
            }
            if entries.len() >= usize::from(self.inner.capabilities.limits.max_sessions) {
                return Err(BrowserDriverError::LimitExceeded);
            }
            entries.insert(entry.request.session_id.clone(), Arc::clone(&entry));
        }
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(operations::open(
            Arc::clone(&self.inner),
            Arc::clone(&entry),
            sender,
        ));
        wait::response(
            Arc::clone(&self.inner),
            entry,
            receiver,
            control,
            self.inner.capabilities.limits.navigation_timeout_ms,
        )
        .await
    }

    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        let entry = self
            .entry(&command.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::command_for_open(&command, &entry.request, &self.inner.capabilities)?;
        let permit = Arc::clone(&self.inner.actions)
            .try_acquire_owned()
            .map_err(|_| BrowserDriverError::LimitExceeded)?;
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(operations::execute(
            Arc::clone(&self.inner),
            Arc::clone(&entry),
            command,
            permit,
            sender,
        ));
        wait::response(
            Arc::clone(&self.inner),
            entry,
            receiver,
            control,
            self.inner.capabilities.limits.action_timeout_ms,
        )
        .await
    }

    async fn cancel_session(&self, id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        let Some(entry) = self.entry(id)? else {
            return Ok(());
        };
        entry.admission_fenced.store(true, Ordering::Release);
        cleanup::acknowledged(cleanup::start(Arc::clone(&self.inner), entry)?).await
    }

    async fn capture(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        self.capture_private(command, control).await
    }
    async fn read_screenshot_chunk(
        &self,
        request: BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        self.read_screenshot_private(request, control).await
    }
    async fn prepare_upload(
        &self,
        request: BrowserUploadPrepareRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.prepare_upload_private(request, control).await
    }
    async fn write_upload_chunk(
        &self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.write_upload_private(request, control).await
    }
    async fn commit_upload(
        &self,
        request: BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.commit_upload_private(request, control).await
    }
    async fn download(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        self.download_private(command, control).await
    }
    async fn read_download_chunk(
        &self,
        request: BrowserDownloadReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        self.read_download_private(request, control).await
    }
    async fn confirm_native_handoff(
        &self,
        request: BrowserNativeHandoffRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.confirm_handoff_private(request, control).await
    }

    async fn close_session(&self, id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        let Some(entry) = self.entry(id)? else {
            return Ok(());
        };
        entry.admission_fenced.store(true, Ordering::Release);
        cleanup::acknowledged(cleanup::start(Arc::clone(&self.inner), Arc::clone(&entry))?).await?;
        let mut entries = self
            .inner
            .entries
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if entries
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, &entry))
        {
            entries.remove(id);
        }
        Ok(())
    }
}

impl Drop for BrowserHostPool {
    fn drop(&mut self) {
        if let Ok(entries) = self.inner.entries.lock() {
            for entry in entries.values() {
                entry.authority.cancel();
                entry.admission_fenced.store(true, Ordering::Release);
                let _ = cleanup::start(Arc::clone(&self.inner), Arc::clone(entry));
            }
        }
    }
}
