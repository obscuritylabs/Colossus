//! Native-only enrollment on supervisor-created inherited Unix channels.
use std::{
    ffi::CString,
    io::{Read as _, Write as _},
    os::{
        fd::{AsRawFd as _, FromRawFd as _, IntoRawFd as _},
        unix::{
            ffi::OsStrExt as _,
            fs::{DirBuilderExt as _, MetadataExt as _},
            net::UnixStream,
        },
    },
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use colossus_browser_bridge::{
    BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel, serve_browser_host,
};
use colossus_ports::BrowserDriverError;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    arguments::NativeArguments, cef, ffi, platform, presentation, provision, queue, runtime,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    enrollment: BrowserBridgeEnrollment,
    profile_path: PathBuf,
    #[serde(default)]
    persistent_profile: Option<colossus_contracts::BrowserProfileId>,
    proxy: ProxyConfiguration,
    pki: Option<provision::Configuration>,
    presentation: Option<PresentationConfiguration>,
    #[cfg(all(target_os = "linux", feature = "native-custody-test"))]
    custody_test_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PresentationConfiguration {
    human_input: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProxyConfiguration {
    address: String,
    port: u16,
    username: String,
    password: String,
}
impl Drop for ProxyConfiguration {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.username.zeroize();
        self.password.zeroize();
    }
}

// Categorical progress uses only the already peer-checked bootstrap socket.
// These fixed bytes contain no configuration, page data, or capability grant.
struct StartupReceipt {
    stream: UnixStream,
    phase: u8,
    initialized: bool,
}
impl StartupReceipt {
    fn progress(&mut self, phase: u8) {
        self.phase = phase;
        let _ = self.stream.write_all(&[b'C', b'B', b'H', 1, phase]);
    }
}
impl Drop for StartupReceipt {
    fn drop(&mut self) {
        if !self.initialized {
            let _ = self
                .stream
                .write_all(&[b'C', b'B', b'H', 1, 0x80 | self.phase]);
        }
    }
}

fn configure_proxy(proxy: &ProxyConfiguration) -> Result<(), BrowserDriverError> {
    let address = CString::new(proxy.address.as_str()).map_err(|_| BrowserDriverError::Denied)?;
    let mut username = proxy.username.as_bytes().to_vec();
    username.push(0);
    let mut password = proxy.password.as_bytes().to_vec();
    password.push(0);
    let username = Zeroizing::new(username);
    let password = Zeroizing::new(password);
    if proxy.username.contains('\0') || proxy.password.contains('\0') {
        return Err(BrowserDriverError::Denied);
    }
    // SAFETY: bounded native bootstrap strings remain alive; configure copies only exact proxy credentials.
    if unsafe {
        ffi::colossus_cef_proxy_configure(
            address.as_ptr(),
            proxy.port,
            username.as_ptr().cast(),
            password.as_ptr().cast(),
        )
    } != 0
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

fn arguments() -> Result<NativeArguments, BrowserDriverError> {
    NativeArguments::new(std::env::args_os().map(|argument| argument.as_bytes().to_vec()))
}

fn options(
    arguments: &mut NativeArguments,
    callbacks: &cef::Callbacks,
    profile: Option<&CString>,
    entry: Option<&platform::Entry>,
) -> Result<ffi::Options, BrowserDriverError> {
    let (argc, argv) = arguments.as_ffi();
    Ok(ffi::Options {
        abi_version: ffi::ABI_VERSION,
        argc,
        argv,
        platform_instance: 0,
        sandbox_info: std::ptr::null_mut(),
        root_cache_path: profile.map_or(std::ptr::null(), |profile| profile.as_ptr()),
        browser_subprocess_path: entry
            .and_then(|entry| entry.helper.as_ref())
            .map_or(std::ptr::null(), |helper| helper.as_ptr()),
        headless: entry.map_or(1, |entry| entry.native_mode),
        callbacks: callbacks.ffi(),
    })
}

fn inherited(descriptor: i32) -> Result<UnixStream, BrowserDriverError> {
    if descriptor < 3 {
        return Err(BrowserDriverError::Denied);
    }
    let mut kind: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
    // SAFETY: bounded getsockopt writes exactly an integer to valid stack storage.
    if unsafe {
        libc::getsockopt(
            descriptor,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            std::ptr::from_mut(&mut kind).cast(),
            &mut length,
        )
    } != 0
        || kind != libc::SOCK_STREAM
    {
        return Err(BrowserDriverError::Denied);
    }
    let mut address: libc::sockaddr_storage = unsafe {
        // SAFETY: all-zero sockaddr storage is a valid output buffer for getsockname.
        std::mem::zeroed()
    };
    let mut length = std::mem::size_of_val(&address) as libc::socklen_t;
    // SAFETY: length describes valid sockaddr storage; descriptor has proved socket type.
    if unsafe {
        libc::getsockname(
            descriptor,
            std::ptr::from_mut(&mut address).cast(),
            &mut length,
        )
    } != 0
        || i32::from(address.ss_family) != libc::AF_UNIX
    {
        return Err(BrowserDriverError::Denied);
    }
    // SAFETY: descriptors are uniquely nominated by the supervisor and taken exactly once.
    let stream = unsafe { UnixStream::from_raw_fd(descriptor) };
    // Helpers must not inherit authentication/control channels across exec.
    // SAFETY: setting descriptor flags has no pointer arguments and stream owns this live fd.
    if unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(BrowserDriverError::Denied);
    }
    Ok(stream)
}

fn profile(configuration: &Configuration) -> Result<CString, BrowserDriverError> {
    let path = &configuration.profile_path;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| BrowserDriverError::Unavailable)?;
    // SAFETY: geteuid has no parameters and reads the current process effective UID.
    let owner = unsafe { libc::geteuid() };
    if !path.is_absolute()
        || !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || path
            .canonicalize()
            .map_err(|_| BrowserDriverError::Denied)?
            != *path
    {
        return Err(BrowserDriverError::Denied);
    }
    CString::new(path.as_os_str().as_bytes()).map_err(|_| BrowserDriverError::Denied)
}

fn create_container_profile(configuration: &Configuration) -> Result<(), BrowserDriverError> {
    let parent = std::path::Path::new("/var/colossus-browser");
    if configuration.profile_path != parent.join("profile") {
        return Err(BrowserDriverError::Denied);
    }
    let metadata =
        std::fs::symlink_metadata(parent).map_err(|_| BrowserDriverError::Unavailable)?;
    // SAFETY: geteuid reads process metadata and takes no parameters.
    let owner = unsafe { libc::geteuid() };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || parent
            .canonicalize()
            .map_err(|_| BrowserDriverError::Denied)?
            != parent
    {
        return Err(BrowserDriverError::Denied);
    }
    if configuration.persistent_profile.is_some() {
        // Only the supervisor's exact owned profile lease may supply this mount.
        // Never create or silently substitute a fresh temporary profile here.
        profile(configuration)?;
        return Ok(());
    }
    // The fresh supervisor-created private tmpfs has no prior browser profile.
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&configuration.profile_path)
        .map_err(|_| BrowserDriverError::Denied)
}

pub fn run() -> Result<(), BrowserDriverError> {
    // SAFETY: this dedicated Unix process has not initialized CEF or spawned
    // any worker threads. Its helpers inherit private permissions for newly
    // created profile, NSS and download files; no Desktop process is modified.
    unsafe { libc::umask(0o077) };
    let mut arguments = arguments()?;
    let callbacks = Box::<cef::Callbacks>::default();
    // Chromium reexecs this executable for sandboxed helpers. Dispatch before
    // inspecting parent-only bootstrap descriptors; those are close-on-exec.
    if std::env::args_os()
        .skip(1)
        .any(|argument| argument.as_bytes().starts_with(b"--type="))
    {
        // macOS helpers have their own scoped-sandbox bundle entry; the Rust
        // browser host must never re-enter Chromium as an unsigned helper.
        #[cfg(target_os = "macos")]
        return Err(BrowserDriverError::Denied);
        #[cfg(target_os = "linux")]
        {
            let options = options(&mut arguments, &callbacks, None, None)?;
            let mut exit = -1;
            // SAFETY: main thread bootstrap borrows arguments and callbacks through helper exit.
            let status = unsafe { ffi::colossus_cef_bootstrap(&options, &mut exit) };
            if status != 0 || exit < 0 {
                return Err(BrowserDriverError::Unavailable);
            }
            std::process::exit(exit);
        }
    }
    let startup = std::env::args_os().skip(1).collect::<Vec<_>>();
    let container_presentation =
        startup == [std::ffi::OsString::from("--oci-presentation-sockets")];
    let is_container =
        container_presentation || startup == [std::ffi::OsString::from("--oci-sockets")];
    let streams = if is_container {
        platform::container_channels(container_presentation)?
    } else {
        let descriptors: Vec<i32> = std::env::args_os()
            .skip(1)
            .map(|argument| {
                argument
                    .to_str()
                    .ok_or(BrowserDriverError::Denied)?
                    .parse::<i32>()
                    .map_err(|_| BrowserDriverError::Denied)
            })
            .collect::<Result<_, _>>()?;
        if !(3..=4).contains(&descriptors.len())
            || descriptors
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != descriptors.len()
        {
            return Err(BrowserDriverError::Denied);
        }
        descriptors
            .into_iter()
            .map(inherited)
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut streams = streams.into_iter();
    let bootstrap = streams.next().ok_or(BrowserDriverError::Denied)?;
    let data = streams.next().ok_or(BrowserDriverError::Denied)?;
    let control = streams.next().ok_or(BrowserDriverError::Denied)?;
    let presentation_stream = streams
        .next()
        .map(|stream| inherited(stream.into_raw_fd()))
        .transpose()?;
    let mut bootstrap = inherited(bootstrap.into_raw_fd())?;
    let data = inherited(data.into_raw_fd())?;
    let control = inherited(control.into_raw_fd())?;
    let mut kept = vec![bootstrap.as_raw_fd(), data.as_raw_fd(), control.as_raw_fd()];
    if let Some(stream) = &presentation_stream {
        kept.push(stream.as_raw_fd());
    }
    platform::close_unrelated_descriptors(&kept)?;
    // try_clone produces a close-on-exec fd for this same private channel,
    // after closing unrelated fds and before any native/helper thread exists.
    let mut receipt = StartupReceipt {
        stream: bootstrap
            .try_clone()
            .map_err(|_| BrowserDriverError::Unavailable)?,
        phase: 0,
        initialized: false,
    };
    receipt.progress(1);
    bootstrap
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| BrowserDriverError::Failed)?;
    let mut key = Zeroizing::new([0; 32]);
    bootstrap
        .read_exact(key.as_mut())
        .map_err(|_| BrowserDriverError::Denied)?;
    if *key == [0; 32] {
        return Err(BrowserDriverError::Denied);
    }
    let mut length = [0; 4];
    bootstrap
        .read_exact(&mut length)
        .map_err(|_| BrowserDriverError::Denied)?;
    let length = usize::try_from(u32::from_be_bytes(length))
        .map_err(|_| BrowserDriverError::LimitExceeded)?;
    if length == 0 || length > 32 * 1024 {
        return Err(BrowserDriverError::LimitExceeded);
    }
    let mut configuration_bytes = Zeroizing::new(vec![0; length]);
    bootstrap
        .read_exact(&mut configuration_bytes)
        .map_err(|_| BrowserDriverError::Denied)?;
    let configuration: Configuration =
        serde_json::from_slice(&configuration_bytes).map_err(|_| BrowserDriverError::Denied)?;
    drop(bootstrap);
    configuration.enrollment.validate()?;
    let selected_profile = configuration.persistent_profile.as_ref().map_or(
        colossus_contracts::BrowserProfileSelection::Temporary,
        |id| colossus_contracts::BrowserProfileSelection::Workspace { id: id.clone() },
    );
    if selected_profile != configuration.enrollment.profile {
        return Err(BrowserDriverError::Denied);
    }
    receipt.progress(2);
    if configuration.presentation.is_some() != presentation_stream.is_some()
        || (configuration.enrollment.mode != colossus_contracts::BrowserMode::Headless
            && !(configuration.enrollment.mode == colossus_contracts::BrowserMode::Embedded
                && configuration.presentation.is_some()))
        || configuration
            .presentation
            .as_ref()
            .is_some_and(|presentation| {
                presentation.human_input
                    && configuration.enrollment.mode != colossus_contracts::BrowserMode::Embedded
            })
    {
        return Err(BrowserDriverError::Unsupported);
    }
    let entry = platform::Entry::prepare(
        configuration.enrollment.mode,
        &configuration.enrollment.capabilities,
    )?;
    if is_container {
        create_container_profile(&configuration)?;
    }
    let cache = profile(&configuration)?;
    receipt.progress(3);
    let (_native_home, identity_policy) =
        provision::Home::prepare(&configuration.profile_path, configuration.pki.as_ref())?;
    receipt.progress(4);
    #[cfg(target_os = "linux")]
    if let Some(pki) = &configuration.pki {
        receipt
            .stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| BrowserDriverError::Failed)?;
        let mut acknowledgement = [0; 5];
        receipt
            .stream
            .read_exact(&mut acknowledgement)
            .map_err(|_| BrowserDriverError::Denied)?;
        if acknowledgement != [b'C', b'B', b'H', 1, 0x84] {
            return Err(BrowserDriverError::Denied);
        }
        pki.confirm_source_retirement()?;
    }
    #[cfg(all(target_os = "linux", feature = "native-custody-test"))]
    if let Some(url) = &configuration.custody_test_url {
        let fixture =
            colossus_contracts::BrowserUrl::parse(url).map_err(|_| BrowserDriverError::Denied)?;
        if !configuration
            .enrollment
            .allowed_origins
            .contains(&fixture.origin())
        {
            return Err(BrowserDriverError::Denied);
        }
        let url = CString::new(url.as_str()).map_err(|_| BrowserDriverError::Denied)?;
        // SAFETY: OFF-default native test build; bounded reviewed fixture URL and
        // wholly owned NSS HOME exist before any Chromium/helper thread starts.
        if unsafe { ffi::colossus_cef_custody_test_prepare(url.as_ptr()) } != 0 {
            return Err(BrowserDriverError::Denied);
        }
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    callbacks.configure_identity(identity_policy, Arc::clone(&cancelled))?;
    let mut relay = if is_container {
        if configuration.proxy.address != "127.0.0.1" || configuration.proxy.port != 18081 {
            return Err(BrowserDriverError::Denied);
        }
        Some(platform::Relay::start(Arc::clone(&cancelled))?)
    } else {
        None
    };
    configure_proxy(&configuration.proxy)?;
    // SAFETY: fixed native-only opt-in before CEF starts; root cache path is the
    // verified supervisor-owned directory, never a model or renderer selection.
    if unsafe {
        ffi::colossus_cef_profile_persistent(i32::from(configuration.persistent_profile.is_some()))
    } != 0
    {
        return Err(BrowserDriverError::Denied);
    }
    receipt.progress(5);
    let options = options(&mut arguments, &callbacks, Some(&cache), Some(&entry))?;
    let mut subprocess = -1;
    receipt.progress(6);
    // SAFETY: main-thread initialization; arguments/profile/callback storage outlive shutdown.
    if unsafe { ffi::colossus_cef_bootstrap(&options, &mut subprocess) } != 0 || subprocess >= 0 {
        return Err(BrowserDriverError::Unavailable);
    }
    receipt.progress(7);
    receipt.initialized = true;
    drop(receipt);
    let finished = Arc::new(AtomicBool::new(false));
    let presentation_revoked = Arc::new(AtomicBool::new(false));
    let (presentation_sender, presentation_receiver) = tokio::sync::mpsc::channel(16);
    let presenter = Arc::new(presentation::Adapter {
        sender: presentation_sender,
        revoked: Arc::clone(&presentation_revoked),
    });
    let presentation_enabled = configuration.presentation.is_some();
    let human_input = configuration
        .presentation
        .as_ref()
        .is_some_and(|presentation| presentation.human_input);
    let enrollment_digest: [u8; 32] = Sha256::digest(
        serde_json::to_vec(&configuration.enrollment).map_err(|_| BrowserDriverError::Denied)?,
    )
    .into();
    let key = BrowserBridgeKey::from_bootstrap(key);
    let presentation_key = key.derive_presentation_key();
    let (data_sender, data_receiver) = tokio::sync::mpsc::channel(8);
    let (control_sender, control_receiver) = tokio::sync::mpsc::channel(4);
    let driver = Arc::new(queue::Driver {
        profile: selected_profile,
        capabilities: configuration.enrollment.capabilities.clone(),
        data: data_sender,
        control: control_sender,
        cancelled: Arc::clone(&cancelled),
    });
    let thread_finished = Arc::clone(&finished);
    let transfer_profile = configuration.profile_path.clone();
    let endpoint = std::thread::spawn(move || {
        struct Finished(Arc<AtomicBool>);
        impl Drop for Finished {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _finished = Finished(thread_finished);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| BrowserDriverError::Failed)?;
        runtime.block_on(async move {
            let presentation_endpoint = if let Some(stream) = presentation_stream {
                stream
                    .set_nonblocking(true)
                    .map_err(|_| BrowserDriverError::Failed)?;
                let stream = tokio::net::UnixStream::from_std(stream)
                    .map_err(|_| BrowserDriverError::Failed)?;
                let (reader, writer) = stream.into_split();
                Some(tokio::spawn(
                    colossus_browser_presentation::serve_presentation(
                        colossus_browser_presentation::PresentationChannel::new(reader, writer),
                        presentation_key,
                        enrollment_digest,
                        presenter,
                    ),
                ))
            } else {
                None
            };
            data.set_nonblocking(true)
                .map_err(|_| BrowserDriverError::Failed)?;
            control
                .set_nonblocking(true)
                .map_err(|_| BrowserDriverError::Failed)?;
            let data =
                tokio::net::UnixStream::from_std(data).map_err(|_| BrowserDriverError::Failed)?;
            let control = tokio::net::UnixStream::from_std(control)
                .map_err(|_| BrowserDriverError::Failed)?;
            let (data_reader, data_writer) = data.into_split();
            let (control_reader, control_writer) = control.into_split();
            let result = serve_browser_host(
                InheritedBrowserChannel::new(data_reader, data_writer),
                InheritedBrowserChannel::new(control_reader, control_writer),
                configuration.enrollment,
                key,
                driver,
            )
            .await;
            if let Some(endpoint) = presentation_endpoint {
                endpoint.abort();
                let _ = endpoint.await;
            }
            result
        })
    });
    let mut host = cef::Host::new(callbacks, Arc::clone(&cancelled));
    host.configure_transfers(&transfer_profile)?;
    if presentation_enabled {
        host.enable_presentation(human_input);
    }
    runtime::pump(
        &mut host,
        runtime::Channels {
            data: data_receiver,
            control: control_receiver,
            presentation: presentation_receiver,
        },
        runtime::Lifetime {
            finished,
            presentation_revoked,
            cancelled,
        },
        || relay.as_mut().map_or(Ok(()), platform::Relay::stop),
    )?;
    endpoint
        .join()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)??;
    _native_home.finish()?;
    Ok(())
}
