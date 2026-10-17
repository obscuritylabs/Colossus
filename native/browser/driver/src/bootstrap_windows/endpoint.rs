//! Always join native pipe workers and the private endpoint before DLL return.
use std::{
    os::windows::io::AsRawHandle as _,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use colossus_browser_bridge::{
    BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel, serve_browser_host,
};
use colossus_ports::BrowserDriverError;
use windows_sys::Win32::{Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject};
use zeroize::Zeroizing;

use crate::{presentation, queue, windows_io::Pipe};

pub(super) struct Request {
    pub data: Pipe,
    pub control: Pipe,
    pub presentation: Option<Pipe>,
    pub enrollment: BrowserBridgeEnrollment,
    pub key: BrowserBridgeKey,
    pub presentation_key: Zeroizing<[u8; 32]>,
    pub digest: [u8; 32],
    pub driver: Arc<queue::Driver>,
    pub presenter: Arc<presentation::Adapter>,
    pub finished: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
}
pub(super) struct Endpoint(Option<JoinHandle<Result<(), BrowserDriverError>>>);
struct Finished(Arc<AtomicBool>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) fn start(request: Request) -> Result<Endpoint, BrowserDriverError> {
    std::thread::Builder::new()
        .name("colossus-browser-private-endpoint".into())
        .spawn(move || {
            let _finished = Finished(Arc::clone(&request.finished));
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|_| BrowserDriverError::Failed)?;
            runtime.block_on(serve(request))
        })
        .map(|join| Endpoint(Some(join)))
        .map_err(|_| BrowserDriverError::Unavailable)
}
async fn cancelled(token: Arc<AtomicBool>) {
    while !token.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn serve(request: Request) -> Result<(), BrowserDriverError> {
    let presenter = request.presentation.map(|pipe| {
        tokio::spawn(colossus_browser_presentation::serve_presentation(
            colossus_browser_presentation::PresentationChannel::new(pipe.reader, pipe.writer),
            request.presentation_key,
            request.digest,
            request.presenter,
        ))
    });
    let result = tokio::select! {
        result = serve_browser_host(InheritedBrowserChannel::new(request.data.reader, request.data.writer),
            InheritedBrowserChannel::new(request.control.reader, request.control.writer),
            request.enrollment, request.key, request.driver) => result,
        () = cancelled(request.stop) => Err(BrowserDriverError::OutcomeUnknown),
    };
    if let Some(presenter) = presenter {
        presenter.abort();
        let _ = presenter.await;
    }
    // Every owned anonymous pipe half drops here; its native synchronous worker
    // cancels and joins before runtime tasks and this client DLL can be unloaded.
    result
}
impl Endpoint {
    pub(super) fn finish(mut self) -> Result<(), BrowserDriverError> {
        let join = self.0.take().ok_or(BrowserDriverError::OutcomeUnknown)?;
        for _ in 0..1000 {
            // SAFETY: JoinHandle retains this exact native thread; no PID lookup
            // or unrelated thread can satisfy the signaled-state cleanup barrier.
            if unsafe { WaitForSingleObject(join.as_raw_handle(), 10) } == WAIT_OBJECT_0 {
                return join
                    .join()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            }
        }
        // Returning to CEF's loader while a worker can execute Rust would allow
        // use after DLL unload. Parent must retain unacknowledged whole-Job cleanup.
        std::process::abort();
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if self.0.is_some() {
            std::process::abort();
        }
    }
}
