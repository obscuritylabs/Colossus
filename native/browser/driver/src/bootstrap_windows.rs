//! Supported CEF bootstrap thread and supervisor-only anonymous pipe ownership.
mod configuration;
mod endpoint;

use std::{
    ffi::CString,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use colossus_browser_bridge::BrowserBridgeKey;
use colossus_ports::BrowserDriverError;
use colossus_windows_native::BoundPath;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::{cef, ffi, identity, presentation, queue, runtime, windows_io};

struct Initialized(cef::Host);
impl Drop for Initialized {
    fn drop(&mut self) {
        // Catching a Rust panic or late startup error cannot let native callbacks
        // dangle while CEF still runs. Unproved shutdown must never return to the
        // bootstrap loader: its supervising parent retains whole-Job cleanup.
        if self.0.shutdown().is_err() {
            std::process::abort();
        }
    }
}

pub fn run() -> Result<(), BrowserDriverError> {
    let pipes = windows_io::inherited(&std::env::args_os().skip(1).collect::<Vec<_>>())?;
    let _workers = windows_io::Workers::retain(&pipes);
    let mut pipes = pipes.into_iter();
    let mut bootstrap = pipes.next().ok_or(BrowserDriverError::Denied)?;
    let data = pipes.next().ok_or(BrowserDriverError::Denied)?;
    let control = pipes.next().ok_or(BrowserDriverError::Denied)?;
    let presenter_pipe = pipes.next();
    let startup = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| BrowserDriverError::Unavailable)?;
    let receipt = |writer: &mut windows_io::Writer, phase| {
        startup.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(5),
                writer.write_all(&[b'C', b'B', b'H', 1, phase]),
            )
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .map_err(|_| BrowserDriverError::OutcomeUnknown)
        })
    };
    receipt(&mut bootstrap.writer, 1)?;
    let (key, configuration) = startup.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            configuration::read(&mut bootstrap.reader),
        )
        .await
        .map_err(|_| BrowserDriverError::Denied)?
    })?;
    configuration.validate(presenter_pipe.is_some())?;
    receipt(&mut bootstrap.writer, 2)?;
    let (_profile, cache) = configuration.profile()?;
    receipt(&mut bootstrap.writer, 3)?;
    let executable = std::env::current_exe().map_err(|_| BrowserDriverError::Unavailable)?;
    let helper = executable
        .parent()
        .ok_or(BrowserDriverError::Denied)?
        .join("colossus-browser-helper.exe");
    let _helper = BoundPath::open_file(&helper).map_err(|_| BrowserDriverError::Denied)?;
    let helper = CString::new(helper.to_str().ok_or(BrowserDriverError::Denied)?)
        .map_err(|_| BrowserDriverError::Denied)?;
    let callbacks = Box::<cef::Callbacks>::default();
    let cancelled = Arc::new(AtomicBool::new(false));
    callbacks.configure_identity(identity::Policy::new(Vec::new())?, Arc::clone(&cancelled))?;
    receipt(&mut bootstrap.writer, 4)?;
    configuration.proxy()?;
    receipt(&mut bootstrap.writer, 5)?;
    let mut instance = 0;
    let mut sandbox = std::ptr::null_mut();
    // SAFETY: native RunWinMain retains both values on this exact bootstrap thread.
    if unsafe { ffi::colossus_cef_windows_context(&mut instance, &mut sandbox) } != 0 {
        return Err(BrowserDriverError::Denied);
    }
    let mut arguments = [std::ptr::null_mut()];
    let options = ffi::Options {
        abi_version: ffi::ABI_VERSION,
        argc: 0,
        argv: arguments.as_mut_ptr(),
        platform_instance: instance,
        sandbox_info: sandbox,
        root_cache_path: cache.as_ptr(),
        browser_subprocess_path: helper.as_ptr(),
        headless: 2,
        callbacks: callbacks.ffi(),
    };
    let mut subprocess = -1;
    receipt(&mut bootstrap.writer, 6)?;
    // SAFETY: native main-thread entry; boxed callbacks, retained paths, sandbox
    // and profile cache strings live through actual CefShutdown below.
    if unsafe { ffi::colossus_cef_bootstrap(&options, &mut subprocess) } != 0 || subprocess >= 0 {
        return Err(BrowserDriverError::Unavailable);
    }
    let mut initialized = Initialized(cef::Host::new(callbacks, Arc::clone(&cancelled)));
    let host = &mut initialized.0;
    host.configure_transfers(&configuration.profile_path)?;
    let started = receipt(&mut bootstrap.writer, 7);
    drop(bootstrap);
    drop(startup);
    if let Err(error) = started {
        host.shutdown()?;
        return Err(error);
    }
    let human_input = configuration
        .presentation
        .as_ref()
        .is_some_and(|value| value.human_input);
    host.enable_presentation(human_input);
    let digest = Sha256::digest(
        serde_json::to_vec(&configuration.enrollment).map_err(|_| BrowserDriverError::Denied)?,
    )
    .into();
    let key = BrowserBridgeKey::from_bootstrap(key);
    let presentation_key = key.derive_presentation_key();
    let finished = Arc::new(AtomicBool::new(false));
    let endpoint_stop = Arc::new(AtomicBool::new(false));
    let revoked = Arc::new(AtomicBool::new(false));
    let (presentation_sender, presentation_receiver) = tokio::sync::mpsc::channel(16);
    let presenter = Arc::new(presentation::Adapter {
        sender: presentation_sender,
        revoked: Arc::clone(&revoked),
    });
    let (data_sender, data_receiver) = tokio::sync::mpsc::channel(8);
    let (control_sender, control_receiver) = tokio::sync::mpsc::channel(4);
    let driver = Arc::new(queue::Driver {
        profile: configuration.enrollment.profile.clone(),
        capabilities: configuration.enrollment.capabilities.clone(),
        data: data_sender,
        control: control_sender,
        cancelled: Arc::clone(&cancelled),
    });
    let endpoint = endpoint::start(endpoint::Request {
        data,
        control,
        presentation: presenter_pipe,
        enrollment: configuration.enrollment,
        key,
        presentation_key,
        digest,
        driver,
        presenter,
        finished: Arc::clone(&finished),
        stop: Arc::clone(&endpoint_stop),
    })?;
    let result = runtime::pump(
        host,
        runtime::Channels {
            data: data_receiver,
            control: control_receiver,
            presentation: presentation_receiver,
        },
        runtime::Lifetime {
            finished,
            presentation_revoked: revoked,
            cancelled: Arc::clone(&cancelled),
        },
        || Ok(()),
    );
    // Always stop/join private endpoint I/O before returning to CEF's loader,
    // including failed native shutdown. Profile deletion belongs to the parent
    // only after its actual whole-Job-empty and network-filter cleanup barriers.
    cancelled.store(true, std::sync::atomic::Ordering::Release);
    endpoint_stop.store(true, std::sync::atomic::Ordering::Release);
    let endpoint = endpoint.finish();
    result.and(endpoint)
}
