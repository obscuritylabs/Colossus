//! Independent one-shot human capture; no ComputerBinding, run or input permit.
use super::{Error, identity, read_worker::ReadWorker};
use crate::{ForegroundAppshot, HumanAppshot, NativeProcessIdentity, TrustedComputerContext, wire};
use serde::{Deserialize, Serialize};
use std::{
    os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    },
};

use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
static CAPTURING: AtomicBool = AtomicBool::new(false);
static QUARANTINED: AtomicBool = AtomicBool::new(false);
static RETAINED_WORKER: OnceLock<Mutex<Option<ReadWorker>>> = OnceLock::new();
struct CaptureGuard;
impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURING.store(false, Ordering::Release);
    }
}
fn retain_unknown(worker: ReadWorker) -> Error {
    QUARANTINED.store(true, Ordering::Release);
    if let Ok(mut retained) = RETAINED_WORKER.get_or_init(|| Mutex::new(None)).lock() {
        *retained = Some(worker);
    } else {
        // Never close uncertain native custody merely because a Rust lock was
        // poisoned. The process owns the Job until operating-system teardown.
        std::mem::forget(worker);
    }
    Error::OutcomeUnknown
}

const MAX_PNG: u32 = 6 * 1024 * 1024;
const MAX_PIXELS: u32 = 16 * 1024 * 1024;
const MAX_TEXT: usize = 32 * 1024;

#[cfg(feature = "native-test-driver")]
pub(crate) mod fixture;

#[derive(Clone, Copy, Serialize, Deserialize)]
enum ReadKind {
    Image,
    Text,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    nonce: String,
    parent: NativeProcessIdentity,
    context: TrustedComputerContext,
    window: identity::Window,
    kind: ReadKind,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    nonce: String,
    result: Result<Option<String>, Error>,
}

pub(crate) fn prepare() -> Result<ForegroundAppshot, Error> {
    identity::ordinary_host()?;
    let context = TrustedComputerContext::for_current_host()?;
    let window = identity::inspect(identity::foreground(), &context)?;
    let title = identity::title(window.hwnd());
    Ok(ForegroundAppshot {
        window,
        context,
        title,
    })
}

pub(crate) fn capture(
    ticket: ForegroundAppshot,
    include_text: bool,
) -> Result<HumanAppshot, Error> {
    if QUARANTINED.load(Ordering::Acquire) {
        return Err(Error::OutcomeUnknown);
    }
    CAPTURING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| Error::Unavailable)?;
    let _guard = CaptureGuard;
    if QUARANTINED.load(Ordering::Acquire) {
        return Err(Error::OutcomeUnknown);
    }
    ticket.window.revalidate(&ticket.context)?;
    let (_, png) = read(&ticket, ReadKind::Image, Duration::from_secs(6))?;
    if png.is_empty() || png.len() > MAX_PNG as usize {
        return Err(Error::Failed);
    }
    // A non-responsive accessibility provider must not withhold valid pixels.
    // Uncertain helper cleanup remains fatal rather than releasing its output.
    let text = if !include_text {
        None
    } else {
        match read(&ticket, ReadKind::Text, Duration::from_secs(3)) {
            Ok((text, binary)) if binary.is_empty() => text,
            Err(Error::OutcomeUnknown) => return Err(Error::OutcomeUnknown),
            _ => None,
        }
    };
    ticket.window.revalidate(&ticket.context)?;
    Ok(HumanAppshot {
        png,
        text,
        window_title: ticket.title,
        width: ticket.window.rect.width,
        height: ticket.window.rect.height,
    })
}

fn read(
    ticket: &ForegroundAppshot,
    kind: ReadKind,
    timeout: Duration,
) -> Result<(Option<String>, Vec<u8>), Error> {
    let path = std::env::current_exe().map_err(|_| Error::Unavailable)?;
    let image = colossus_windows_native::BoundPath::open_immutable_file(&path)
        .map_err(|_| Error::Denied)?;
    let mut command = std::process::Command::new(&path);
    command
        .arg("__appshot-read-v1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    super::configure_environment(&mut command)?;
    colossus_windows_native::configure_suspended_process(&mut command);
    let mut worker = ReadWorker {
        child: command.spawn().map_err(|_| Error::Unavailable)?,
        job: None,
        reader: None,
    };
    let outcome = (|| {
        worker.job = Some(
            colossus_windows_native::KillOnCloseJob::assign_verify_and_resume(
                &worker.child,
                worker.child.id(),
                image.identity(),
            )
            .map_err(|_| Error::Denied)?,
        );
        let nonce = crate::protocol::opaque("as_p_")?;
        let request = Request {
            version: 1,
            nonce: nonce.clone(),
            parent: identity::process(std::process::id())?,
            context: ticket.context.clone(),
            window: ticket.window.clone(),
            kind,
        };
        let mut input = worker.child.stdin.take().ok_or(Error::Failed)?;
        wire::write(&mut input, &request, &[]).map_err(|_| Error::Failed)?;
        drop(input);
        let mut output = worker.child.stdout.take().ok_or(Error::Failed)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        worker.reader = Some(std::thread::spawn(move || {
            let result = wire::read::<Reply>(&mut output);
            let _ = sender.send(result);
        }));
        let (reply, binary) = receiver
            .recv_timeout(timeout)
            .map_err(|_| Error::Unavailable)?
            .map_err(|_| Error::Failed)?;
        if reply.nonce != nonce || binary.len() > MAX_PNG as usize {
            return Err(Error::Failed);
        }
        let text = reply.result?;
        if text.as_ref().is_some_and(|value| value.len() > MAX_TEXT) {
            return Err(Error::Failed);
        }
        match kind {
            ReadKind::Image if binary.is_empty() || text.is_some() => Err(Error::Failed),
            ReadKind::Text if !binary.is_empty() => Err(Error::Failed),
            _ => Ok((text, binary)),
        }
    })();
    if worker.cleanup().is_err() {
        return Err(retain_unknown(worker));
    }
    // Retain the job until a kernel-backed zero-process barrier is observed.
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match worker.job.as_ref().map(|job| job.active_processes()) {
            Some(Ok(0)) => break,
            Some(Ok(_)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            _ => return Err(retain_unknown(worker)),
        }
    }
    outcome
}

fn verify_parent(parent: &NativeProcessIdentity) -> Result<(), Error> {
    let actual = identity::ancestors(std::process::id())?
        .into_iter()
        .next()
        .ok_or(Error::Denied)?;
    if identity::process(actual)? != *parent {
        return Err(Error::Denied);
    }
    // SAFETY: query-only handle; closed after copying the actual parent's image path.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, actual) };
    if raw.is_null() {
        return Err(Error::Denied);
    }
    // SAFETY: successful OpenProcess transfers this handle into RAII custody.
    let process = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    let parent_image = process_image(&process)?;
    let current = std::env::current_exe().map_err(|_| Error::Denied)?;
    let current_image = colossus_windows_native::BoundPath::open_immutable_file(&current)
        .map_err(|_| Error::Denied)?;
    if parent_image.identity() != current_image.identity() || identity::process(actual)? != *parent
    {
        return Err(Error::Denied);
    }
    Ok(())
}

fn process_image(process: &OwnedHandle) -> Result<colossus_windows_native::BoundPath, Error> {
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    // SAFETY: retained query handle and bounded, writable UTF-16 output.
    if unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle().cast::<std::ffi::c_void>() as HANDLE,
            0,
            path.as_mut_ptr(),
            &mut length,
        )
    } == 0
    {
        return Err(Error::Denied);
    }
    let image_path = std::path::PathBuf::from(
        String::from_utf16(&path[..length as usize]).map_err(|_| Error::Denied)?,
    );
    colossus_windows_native::BoundPath::open_immutable_file(&image_path).map_err(|_| Error::Denied)
}

pub(crate) fn entry() -> Result<(), Error> {
    let (request, binary) =
        wire::read::<Request>(&mut std::io::stdin()).map_err(|_| Error::InvalidInput)?;
    if request.version != 1
        || request.nonce.len() != 37
        || !request.nonce.starts_with("as_p_")
        || !binary.is_empty()
        || !request.nonce.as_bytes()[5..]
            .iter()
            .all(u8::is_ascii_hexdigit)
        || !request.context.protected.contains(&request.parent)
    {
        return Err(Error::Denied);
    }
    verify_parent(&request.parent)?;
    // SAFETY: this disposable worker sets its own DPI context before any capture.
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        )
    };
    identity::ordinary_host()?;
    request.window.revalidate(&request.context)?;
    let result = match request.kind {
        ReadKind::Image => {
            super::capture::png(&request.window, &request.context, MAX_PNG, MAX_PIXELS)
                .map(|png| (None, png))
        }
        ReadKind::Text => {
            super::semantics::snapshot(&request.window, &request.context, 256).map(|nodes| {
                let mut text = String::new();
                for node in nodes {
                    if node.is_password || node.name.trim().is_empty() {
                        continue;
                    }
                    let line = format!("{}: {}\n", node.role, node.name);
                    if text.len() + line.len() > MAX_TEXT {
                        break;
                    }
                    text.push_str(&line);
                }
                ((!text.is_empty()).then_some(text), Vec::new())
            })
        }
    };
    request.window.revalidate(&request.context)?;
    let (result, png) = match result {
        Ok((text, png)) => (Ok(text), png),
        Err(error) => (Err(error), Vec::new()),
    };
    wire::write(
        &mut std::io::stdout(),
        &Reply {
            nonce: request.nonce,
            result,
        },
        &png,
    )
    .map_err(|_| Error::Failed)
}
