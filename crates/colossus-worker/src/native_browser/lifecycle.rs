//! Owned endpoint/task drain; dropped caller futures never discard cleanup ownership.
use super::*;
use colossus_contracts::BrowserSessionId;

pub(crate) struct RunningNativeBrowser {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    completed: tokio::sync::oneshot::Receiver<Result<(), WorkerError>>,
}
impl PreparedNativeBrowser {
    pub(crate) fn start(self) -> RunningNativeBrowser {
        let (stop, stopping) = tokio::sync::oneshot::channel();
        let (completed, completion) = tokio::sync::oneshot::channel();
        tokio::spawn(run(self, stopping, completed));
        RunningNativeBrowser {
            stop: Some(stop),
            completed: completion,
        }
    }
}
impl RunningNativeBrowser {
    pub(crate) async fn shutdown(&mut self) -> Result<(), WorkerError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        tokio::time::timeout(Duration::from_secs(30), &mut self.completed)
            .await
            .map_err(|_| WorkerError::BrowserCleanupUnknown)?
            .map_err(|_| WorkerError::BrowserCleanupUnknown)?
    }
}
impl Drop for RunningNativeBrowser {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
async fn run(
    mut prepared: PreparedNativeBrowser,
    mut stopping: tokio::sync::oneshot::Receiver<()>,
    completed: tokio::sync::oneshot::Sender<Result<(), WorkerError>>,
) {
    let mut tasks = tokio::task::JoinSet::new();
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    // Primary credentials can be Pending before inherited activation. Only an already
    // active credential transitioning away from Active triggers whole endpoint teardown.
    let mut activated = false;
    loop {
        tokio::select! {
            biased;
            _ = &mut stopping => break,
            _ = interval.tick() => {
                if prepared.service.authorized() { activated=true; }
                else if activated { break; }
            }
            result = tasks.join_next(), if !tasks.is_empty() => { let _ = result; }
            accepted = prepared.listener.accept() => {
                match accepted {
                    Ok(stream) => if let Ok(slot) = Arc::clone(&prepared.slots).try_acquire_owned() {
                        let service=Arc::clone(&prepared.service);
                        tasks.spawn(async move { let _slot=slot; let _ = serve::handle(stream,service).await; });
                    },
                    // A rejected kernel peer cannot kill the authenticated owner's endpoint.
                    Err(WorkerError::Protocol(_)) => {},
                    Err(_) => break,
                }
            }
        }
    }
    prepared.service.stopped.store(true, Ordering::Release);
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let result = cleanup(&mut prepared).await;
    let clean = result.is_ok();
    let _ = completed.send(result);
    if !clean {
        // An uncertain result reports failure, while this independent task retains the
        // original listener inode and Runtime authority until exact cleanup is confirmed.
        while cleanup(&mut prepared).await.is_err() {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
async fn cleanup(prepared: &mut PreparedNativeBrowser) -> Result<(), WorkerError> {
    let sessions = prepared
        .service
        .sessions
        .lock()
        .map_err(|_| WorkerError::BrowserCleanupUnknown)?
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut unknown = false;
    for session in sessions {
        if prepared.service.finish(&session, false).await.is_err() {
            unknown = true;
        }
    }
    if prepared.listener.cleanup().is_err() {
        unknown = true;
    }
    if unknown {
        Err(WorkerError::BrowserCleanupUnknown)
    } else {
        Ok(())
    }
}
impl NativeBrowserService {
    pub(super) async fn finish(
        &self,
        session: &BrowserSessionId,
        close: bool,
    ) -> Result<(), WorkerError> {
        let _cleanup = self.cleanup.lock().await;
        if self
            .retired
            .lock()
            .map_err(|_| WorkerError::BrowserCleanupUnknown)?
            .contains(session)
        {
            return Ok(());
        }
        if !close
            && self
                .detached
                .lock()
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                .contains(session)
        {
            return Ok(());
        }
        let tracked = self
            .tracked(session)
            .ok_or_else(|| WorkerError::Protocol("native session is not owned".into()))?;
        let authority = self
            .authority
            .as_ref()
            .ok_or(WorkerError::BrowserCleanupUnknown)?;
        if close {
            self.runtime
                .close_native_browser(authority, session)
                .await
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?;
            self.sessions
                .lock()
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                .remove(session);
            self.detached
                .lock()
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                .remove(session);
            self.retired
                .lock()
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                .insert(session.clone());
        } else {
            self.runtime
                .detach_native_browser(authority, session)
                .await
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?;
            if tracked
                .is_closed()
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?
            {
                self.sessions
                    .lock()
                    .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                    .remove(session);
                self.retired
                    .lock()
                    .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                    .insert(session.clone());
            } else {
                self.detached
                    .lock()
                    .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                    .insert(session.clone());
            }
        }
        Ok(())
    }
}
pub(super) struct DetachOnDrop {
    service: Arc<NativeBrowserService>,
    session: BrowserSessionId,
}
impl DetachOnDrop {
    pub(super) fn new(service: Arc<NativeBrowserService>, session: BrowserSessionId) -> Self {
        Self { service, session }
    }
}
impl Drop for DetachOnDrop {
    fn drop(&mut self) {
        let service = Arc::clone(&self.service);
        let session = self.session.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                while service.finish(&session, false).await.is_err() {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        }
    }
}
