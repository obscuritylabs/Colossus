//! Retained exact native worker identity with repeatable cancellation and join.
use std::{
    os::windows::io::AsRawHandle as _,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::{IO::CancelSynchronousIo, Threading::WaitForSingleObject},
};

pub(super) struct Thread {
    pub(super) cancelled: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
}
impl Thread {
    pub(super) fn start(
        name: &'static str,
        action: impl FnOnce(Arc<AtomicBool>) + Send + 'static,
    ) -> std::io::Result<Self> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let control = Arc::clone(&cancelled);
        let join = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || action(control))?;
        Ok(Self {
            cancelled,
            join: Mutex::new(Some(join)),
        })
    }
    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(join) = self.join.lock()
            && let Some(join) = join.as_ref()
        {
            // SAFETY: JoinHandle retains this exact thread HANDLE, and cancellation
            // never closes it or acts on an input PID/thread ID.
            unsafe {
                CancelSynchronousIo(join.as_raw_handle());
            }
        }
    }
    pub(super) fn drain(&self, timeout: Duration) -> bool {
        self.cancelled.store(true, Ordering::Release);
        let Ok(mut join) = self.join.lock() else {
            return false;
        };
        let Some(held) = join.as_ref() else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        loop {
            // Cancellation can precede ReadFile/WriteFile; repeat while retaining
            // the native handle until the worker actually reaches signaled exit.
            // SAFETY: held owns this exact live thread HANDLE for the entire loop.
            unsafe {
                CancelSynchronousIo(held.as_raw_handle());
            }
            // SAFETY: waiting reads only this exact retained thread's state.
            if unsafe { WaitForSingleObject(held.as_raw_handle(), 10) } == WAIT_OBJECT_0 {
                return join.take().is_some_and(|join| join.join().is_ok());
            }
            if Instant::now() >= deadline {
                return false;
            }
        }
    }
}
impl Drop for Thread {
    fn drop(&mut self) {
        self.cancel();
        if !self.drain(Duration::from_secs(1)) {
            // Drop is never a cleanup acknowledgment. Keep the native thread's
            // handle alive if an incorrect caller abandons an uncertain worker.
            // Supervisors retain PrivateIoLease and retry instead of taking this
            // fallback; client DLLs must fail-stop before unloading on false drain.
            if let Ok(join) = self.join.get_mut()
                && let Some(join) = join.take()
            {
                std::mem::forget(join);
            }
        }
    }
}
