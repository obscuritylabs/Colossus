use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    os::fd::AsRawFd as _,
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use colossus_browser_bridge::{
    BrowserBridgeDriver, BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel,
};
use colossus_contracts::{BrowserMode, BrowserProfileSelection, BrowserSessionId};
use colossus_home::ConfinedRoot;
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tokio::{
    io::AsyncWriteExt as _,
    net::{UnixListener, UnixStream},
    sync::Mutex,
};
use zeroize::Zeroizing;

use super::{
    Installation, engine,
    lifecycle::{Driver, Resources},
    relay,
};
use crate::BrowserEgressLease;

pub(super) async fn owned(
    installation: Arc<Installation>,
    sessions: Arc<Mutex<BTreeMap<BrowserSessionId, Arc<Mutex<Resources>>>>>,
    stopped: Arc<AtomicBool>,
    request: BrowserDriverOpenRequest,
    control: BrowserDriverControl,
) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
    if control.is_cancelled() {
        return Err(BrowserDriverError::Cancelled);
    }
    if request.options.mode != BrowserMode::Headless {
        return Err(BrowserDriverError::Unsupported);
    }
    if request.options.profile != BrowserProfileSelection::Temporary
        && installation.profile_store.is_none()
    {
        return Err(BrowserDriverError::Unsupported);
    }
    let enrolled_pki = installation
        .pki_enrollment
        .as_ref()
        .map(|provider| provider.enroll(&request))
        .transpose()?
        .flatten();
    let material = enrolled_pki.as_ref().or(installation.pki.as_ref());
    let mut capabilities = installation.capabilities.clone();
    let private_ca = material.is_some_and(super::OciBrowserPki::has_private_ca);
    let client_identities =
        material.is_some_and(|pki| pki.has_client_identities_for(&request.options.allowed_origins));
    if (private_ca && !capabilities.private_ca_trust)
        || (client_identities && !capabilities.client_identities)
    {
        // An unavailable PKI ceiling must not silently import trusted material
        // and change certificate behavior while reporting that support is absent.
        return Err(BrowserDriverError::Unavailable);
    }
    capabilities.private_ca_trust &= private_ca;
    capabilities.client_identities &= client_identities;
    let mut instance_nonce = [0; 16];
    getrandom::fill(&mut instance_nonce).map_err(|_| BrowserDriverError::Unavailable)?;
    let enrollment = BrowserBridgeEnrollment {
        binding: request.binding.clone(),
        mode: request.options.mode,
        profile: request.options.profile.clone(),
        allowed_origins: request.options.allowed_origins.clone(),
        instance_nonce,
        component_digest: installation.digest,
        cancellation_closes_context: true,
        capabilities,
    };
    enrollment.validate()?;
    let mut entries = sessions.lock().await;
    if stopped.load(Ordering::Acquire) {
        return Err(BrowserDriverError::Cancelled);
    }
    let nonce = hex::encode(instance_nonce);
    let control_path = installation
        .root
        .prepare_directory(Path::new(&format!("session-{nonce}")))
        .map_err(|_| BrowserDriverError::Unavailable)?;
    let control_root = ConfinedRoot::bind(&control_path).map_err(|_| BrowserDriverError::Denied)?;
    let control_handle = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::DIRECTORY).bits() as i32)
        .open(&control_path)
        .map_err(|_| BrowserDriverError::Denied)?;
    installation.artifacts.admit();
    let resources = Arc::new(Mutex::new(Resources {
        installation: Arc::clone(&installation),
        binding: request.binding.clone(),
        request: request.clone(),
        name: format!("colossus-browser-{nonce}"),
        nonce,
        control: control_root,
        control_handle,
        quarantine: None,
        control_removed: false,
        container: None,
        create_attempted: false,
        process: None,
        egress: None,
        relay: None,
        pki: None,
        profile: None,
        presentation: None,
        closed: false,
        counted: true,
        fenced: Arc::new(AtomicBool::new(false)),
        startup_phase: "prepared",
        shutdown_receipt: super::OciBrowserShutdownReceipt::Unknown,
    }));
    {
        entries.retain(|_, resources| {
            resources
                .try_lock()
                .map_or(true, |resources| !resources.closed)
        });
        if entries.contains_key(&request.session_id) {
            // No process or proxy exists yet; release this known owned directory.
            resources.lock().await.cleanup().await?;
            return Err(BrowserDriverError::Denied);
        }
        if entries.len() >= usize::from(installation.capabilities.limits.max_sessions) {
            resources.lock().await.cleanup().await?;
            return Err(BrowserDriverError::LimitExceeded);
        }
        entries.insert(request.session_id.clone(), Arc::clone(&resources));
    }
    drop(entries);
    let result = start(&resources, enrollment, enrolled_pki, &request, &control).await;
    match result {
        Ok(bridge) => {
            if control.is_cancelled() || stopped.load(Ordering::Acquire) {
                resources.lock().await.cleanup().await?;
                return Err(BrowserDriverError::Cancelled);
            }
            let fenced = Arc::clone(&resources.lock().await.fenced);
            Ok(Arc::new(Driver {
                bridge: Arc::new(bridge),
                resources,
                fenced,
                request: request.clone(),
                binding: request.binding,
                session: request.session_id,
                options: request.options,
            }))
        }
        Err(error) => {
            if resources.lock().await.cleanup().await.is_err() {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            Err(error)
        }
    }
}

async fn start(
    resources: &Arc<Mutex<Resources>>,
    enrollment: BrowserBridgeEnrollment,
    enrolled_pki: Option<super::OciBrowserPki>,
    request: &BrowserDriverOpenRequest,
    control: &BrowserDriverControl,
) -> Result<BrowserBridgeDriver, BrowserDriverError> {
    let held_resources = Arc::clone(resources);
    let mut resources = resources.lock().await;
    if let BrowserProfileSelection::Workspace { id } = &request.options.profile {
        let store = resources
            .installation
            .profile_store
            .as_ref()
            .ok_or(BrowserDriverError::Unsupported)?;
        resources.profile = Some(store.acquire(&request.binding, id).map_err(profile_error)?);
    }
    let path = resources.control.path().to_owned();
    let bootstrap_listener = listener(&resources.control_handle, "bootstrap.sock")?;
    let data_listener = listener(&resources.control_handle, "data.sock")?;
    let control_listener = listener(&resources.control_handle, "control.sock")?;
    let egress_listener = listener(&resources.control_handle, "egress.sock")?;
    let presentation_listener = resources
        .installation
        .presentation
        .then(|| listener(&resources.control_handle, "presentation.sock"))
        .transpose()?;
    resources.pki = enrolled_pki
        .as_ref()
        .or(resources.installation.pki.as_ref())
        .map(|pki| pki.stage(&resources.control, request))
        .transpose()?;
    resources.egress = Some(
        BrowserEgressLease::start(
            request.session_id.clone(),
            enrollment.allowed_origins.clone(),
            resources.installation.limits.egress,
        )
        .await
        .map_err(|_| BrowserDriverError::Unavailable)?,
    );
    resources.startup_phase = "egress_ready";
    let args = engine::arguments(
        &resources.installation,
        &resources.name,
        &resources.nonce,
        &path,
        resources.profile.as_ref(),
    )?;
    if control.is_cancelled() {
        return Err(BrowserDriverError::Cancelled);
    }
    // Retain the ownership nonce before the engine can mutate. If create's response
    // is lost, reconciliation inspects the exact label/image/user and acquires CID.
    resources.create_attempted = true;
    let (created, bytes) = engine::run(&resources.installation, &args).await?;
    let identity = std::str::from_utf8(&bytes)
        .map(str::trim)
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    if !created || !engine::container_id(identity) {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    resources.container = Some(identity.to_owned());
    resources.startup_phase = "created";
    let inspected = engine::inspect(&resources.installation, identity)
        .await?
        .ok_or(BrowserDriverError::OutcomeUnknown)?;
    resources.validate_owned(&inspected)?;
    if control.is_cancelled() {
        return Err(BrowserDriverError::Cancelled);
    }
    let (started, _) = engine::run(
        &resources.installation,
        &["container".into(), "start".into(), identity.to_owned()],
    )
    .await?;
    if !started {
        return Err(BrowserDriverError::Unavailable);
    }
    resources.startup_phase = "started";
    let inspected = engine::inspect(&resources.installation, identity)
        .await?
        .ok_or(BrowserDriverError::OutcomeUnknown)?;
    resources.validate_owned(&inspected)?;
    if !inspected.running || inspected.pid == 0 {
        return Err(BrowserDriverError::Unavailable);
    }
    let process = Arc::new(super::process_identity::ProcessIdentity::bind(
        inspected.pid,
        resources.installation.uid,
        resources.installation.gid,
        identity,
        &resources
            .installation
            .component
            .join("colossus-native-browser-host"),
    )?);
    resources.process = Some(Arc::clone(&process));
    resources.startup_phase = "process_pinned";
    let installation = Arc::clone(&resources.installation);
    let accept = async {
        let (bootstrap, data, control) = tokio::try_join!(
            bootstrap_listener.accept(),
            data_listener.accept(),
            control_listener.accept()
        )
        .map_err(|_| BrowserDriverError::Unavailable)?;
        for stream in [&bootstrap.0, &data.0, &control.0] {
            process.verify_peer(stream)?;
        }
        let presentation = if let Some(listener) = presentation_listener {
            let stream = listener
                .accept()
                .await
                .map_err(|_| BrowserDriverError::Unavailable)?
                .0;
            process.verify_peer(&stream)?;
            Some(stream)
        } else {
            None
        };
        Ok::<_, BrowserDriverError>((bootstrap.0, data.0, control.0, presentation))
    };
    let (mut bootstrap, data, native_control, presentation) =
        tokio::time::timeout(Duration::from_secs(5), accept)
            .await
            .map_err(|_| BrowserDriverError::Unavailable)??;
    resources.startup_phase = "channels_verified";
    let egress = resources
        .egress
        .as_ref()
        .ok_or(BrowserDriverError::OutcomeUnknown)?;
    let password = egress.credential().expose();
    let bytes = Zeroizing::new(
        serde_json::to_vec(&Bootstrap {
            enrollment: &enrollment,
            profile_path: "/var/colossus-browser/profile",
            persistent_profile: match &request.options.profile {
                BrowserProfileSelection::Workspace { id } => Some(id),
                BrowserProfileSelection::Temporary => None,
            },
            pki: resources.pki.as_ref().map(super::pki::StagedPki::bootstrap),
            presentation: installation
                .presentation
                .then_some(Presentation { human_input: false }),
            proxy: Proxy {
                address: "127.0.0.1",
                port: 18081,
                username: "colossus",
                password,
            },
        })
        .map_err(|_| BrowserDriverError::Unavailable)?,
    );
    if bytes.len() > 32 * 1024 {
        return Err(BrowserDriverError::LimitExceeded);
    }
    let address = egress.address();
    let lifetime = installation.limits.egress.lifetime;
    resources.relay = Some(relay::Relay::start(
        egress_listener,
        address,
        Arc::clone(&process),
        lifetime,
    ));
    let mut key = Zeroizing::new([0; 32]);
    getrandom::fill(key.as_mut()).map_err(|_| BrowserDriverError::Unavailable)?;
    process.verify_peer(&bootstrap)?;
    let write = async {
        bootstrap
            .write_all(key.as_ref())
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        bootstrap
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        bootstrap
            .write_all(&bytes)
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        // Keep this private write direction alive for the pre-CEF PKI retirement
        // acknowledgement. Native bootstrap reads the exact bounded length and
        // does not use EOF as configuration framing.
        Ok::<(), BrowserDriverError>(())
    };
    tokio::time::timeout(Duration::from_secs(5), write)
        .await
        .map_err(|_| BrowserDriverError::Unavailable)??;
    resources.startup_phase = "bootstrap_delivered";
    let initialized = tokio::time::timeout(
        Duration::from_millis(u64::from(
            installation.capabilities.limits.navigation_timeout_ms,
        )),
        receipt(&mut bootstrap, &mut resources),
    )
    .await
    .map_err(|_| BrowserDriverError::Unavailable)??;
    if !initialized {
        return Err(BrowserDriverError::Unavailable);
    }
    drop(bootstrap);
    drop(resources);
    let key = BrowserBridgeKey::from_bootstrap(key);
    let presentation_key = key.derive_presentation_key();
    let presentation_digest = Sha256::digest(
        serde_json::to_vec(&enrollment).map_err(|_| BrowserDriverError::Unavailable)?,
    )
    .into();
    let bridge =
        BrowserBridgeDriver::connect(channel(data), channel(native_control), enrollment, key)
            .await?;
    let presentation = if let Some(stream) = presentation {
        let (reader, writer) = stream.into_split();
        let client = colossus_browser_presentation::PresentationClient::connect(
            colossus_browser_presentation::PresentationChannel::new(reader, writer),
            presentation_key,
            presentation_digest,
        )
        .await
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        Some((client, presentation_digest))
    } else {
        None
    };
    let mut resources = held_resources.lock().await;
    if resources.closed || resources.fenced.load(Ordering::Acquire) || control.is_cancelled() {
        if let Some((client, _)) = &presentation {
            client.disconnect();
        }
        bridge.disconnect_for_shutdown();
        return Err(BrowserDriverError::Cancelled);
    }
    resources.presentation = presentation;
    resources.startup_phase = "bridge_ready";
    Ok(bridge)
}

async fn receipt(
    stream: &mut UnixStream,
    resources: &mut Resources,
) -> Result<bool, BrowserDriverError> {
    use tokio::io::AsyncReadExt as _;
    let mut previous = 0_u8;
    for _ in 0..8 {
        let mut record = [0; 5];
        let count = stream
            .read(&mut record[..1])
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        if count == 0 {
            return Ok(previous == 7);
        }
        stream
            .read_exact(&mut record[1..])
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        if record[..4] != [0x43, 0x42, 0x48, 0x01] {
            return Err(BrowserDriverError::Denied);
        }
        let next = record[4];
        if next & 0x80 != 0 {
            if next & 0x7f != previous {
                return Err(BrowserDriverError::Denied);
            }
            return Ok(false);
        }
        if next != previous + 1 || next > 7 {
            return Err(BrowserDriverError::Denied);
        }
        previous = next;
        resources.startup_phase = match next {
            1 => "native_private_fds",
            2 => "native_configuration",
            3 => "native_profile",
            4 => "native_owned_home",
            5 => "native_proxy",
            6 => "native_cef_starting",
            7 => "native_cef_initialized",
            _ => return Err(BrowserDriverError::Denied),
        };
        if next == 4
            && let Some(pki) = resources.pki.as_mut()
        {
            // Native provisioning blocks before CEF initialization. Only exact
            // source retirement plus directory sync permits helpers to start.
            pki.release_inputs()?;
            stream
                .write_all(&[b'C', b'B', b'H', 1, 0x84])
                .await
                .map_err(|_| BrowserDriverError::Unavailable)?;
        }
    }
    Err(BrowserDriverError::LimitExceeded)
}

pub(super) fn listener(
    directory: &std::fs::File,
    name: &str,
) -> Result<UnixListener, BrowserDriverError> {
    if !matches!(
        name,
        "bootstrap.sock" | "data.sock" | "control.sock" | "egress.sock" | "presentation.sock"
    ) {
        return Err(BrowserDriverError::Denied);
    }
    // sockaddr_un is limited to 108 bytes. Binding through the retained directory
    // fd preserves the exact owned socket inode even for deeply nested state paths.
    // The guest connects through its fixed short read-only mount; no /proc path or
    // descriptor is sent to a renderer or trusted native child.
    let path = format!("/proc/self/fd/{}/{}", directory.as_raw_fd(), name);
    let listener = UnixListener::bind(&path).map_err(|_| BrowserDriverError::Unavailable)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| BrowserDriverError::Unavailable)?;
    Ok(listener)
}
fn channel(stream: UnixStream) -> InheritedBrowserChannel {
    let (reader, writer) = stream.into_split();
    InheritedBrowserChannel::new(reader, writer)
}
#[derive(Serialize)]
struct Bootstrap<'a> {
    enrollment: &'a BrowserBridgeEnrollment,
    profile_path: &'a str,
    persistent_profile: Option<&'a colossus_contracts::BrowserProfileId>,
    pki: Option<&'a super::pki::BootstrapPki>,
    presentation: Option<Presentation>,
    proxy: Proxy<'a>,
}

fn profile_error(error: crate::BrowserProfileError) -> BrowserDriverError {
    match error {
        crate::BrowserProfileError::Busy => BrowserDriverError::LimitExceeded,
        crate::BrowserProfileError::OutcomeUnknown => BrowserDriverError::OutcomeUnknown,
        crate::BrowserProfileError::LimitExceeded => BrowserDriverError::LimitExceeded,
        crate::BrowserProfileError::Denied
        | crate::BrowserProfileError::VersionMismatch
        | crate::BrowserProfileError::ResetRequired => BrowserDriverError::Denied,
    }
}
#[derive(Serialize)]
struct Presentation {
    human_input: bool,
}
#[derive(Serialize)]
struct Proxy<'a> {
    address: &'a str,
    port: u16,
    username: &'a str,
    password: &'a str,
}
