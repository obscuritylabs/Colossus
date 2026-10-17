use super::{
    Installation,
    lifecycle::{Driver, Resources},
    startup::Startup,
};
use crate::BrowserEgressLease;
use colossus_browser_bridge::{
    BrowserBridgeDriver, BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel,
};
use colossus_browser_presentation::{PresentationChannel, PresentationClient};
use colossus_contracts::{BrowserMode, BrowserProfileSelection, BrowserSessionId};
use colossus_darwin_process::{
    DarwinAuditPeer, DarwinProcessIdentity, spawn_suspended_private_channels,
};
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    sync::Mutex,
};
use zeroize::Zeroizing;

pub(super) async fn owned(
    installation: Arc<Installation>,
    sessions: Arc<Mutex<BTreeMap<BrowserSessionId, Arc<Mutex<Resources>>>>>,
    stopped: Arc<AtomicBool>,
    request: BrowserDriverOpenRequest,
    control: BrowserDriverControl,
) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
    validate_request(&request, &installation, &control, &stopped)?;
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|_| BrowserDriverError::Unavailable)?;
    if nonce == [0; 16] {
        return Err(BrowserDriverError::Unavailable);
    }
    let enrollment = BrowserBridgeEnrollment {
        public_web: request.options.public_web,
        binding: request.binding.clone(),
        mode: request.options.mode,
        profile: request.options.profile.clone(),
        allowed_origins: request.options.allowed_origins.clone(),
        instance_nonce: nonce,
        component_digest: installation.manifest_digest,
        cancellation_closes_context: true,
        capabilities: installation.capabilities.clone(),
    };
    enrollment.validate()?;
    let fenced = Arc::new(AtomicBool::new(false));
    let resource = Arc::new(Mutex::new(Resources::new(
        request.clone(),
        Arc::clone(&installation),
        Arc::clone(&fenced),
    )));
    {
        let mut entries = sessions.lock().await;
        validate_request(&request, &installation, &control, &stopped)?;
        if !entries.is_empty()
            || entries
                .insert(request.session_id.clone(), Arc::clone(&resource))
                .is_some()
        {
            return Err(BrowserDriverError::Denied);
        }
    }
    let startup = Startup::new(&control, &stopped);
    let result = startup.wait(start(&resource, &enrollment, &startup)).await;
    match result {
        Ok(bridge) if !control.is_cancelled() && !stopped.load(Ordering::Acquire) => {
            Ok(Arc::new(Driver::new(bridge, resource, request, fenced)))
        }
        result => {
            resource.lock().await.cleanup().await?;
            Err(result.err().unwrap_or(BrowserDriverError::Cancelled))
        }
    }
}

fn validate_request(
    request: &BrowserDriverOpenRequest,
    installation: &Installation,
    control: &BrowserDriverControl,
    stopped: &AtomicBool,
) -> Result<(), BrowserDriverError> {
    if control.is_cancelled() || stopped.load(Ordering::Acquire) {
        return Err(BrowserDriverError::Cancelled);
    }
    if request.options.mode != BrowserMode::Embedded
        || !matches!(request.options.profile, BrowserProfileSelection::Temporary)
        || request.options.allowed_origins.len() != 1
    {
        return Err(BrowserDriverError::Unsupported);
    }
    let value = request.options.allowed_origins[0].as_str();
    #[cfg(feature = "macos-browser-tls-development")]
    if let Some(registry) = &installation.pki_registry {
        registry
            .enroll(request)?
            .ok_or(BrowserDriverError::Denied)?;
    }
    if let Some(tls) = &installation.tls_config {
        if request.options.allowed_origins[0] != tls.origin || installation.owned_process.is_none()
        {
            return Err(BrowserDriverError::Denied);
        }
        return super::validate_tls(tls);
    }
    let Some(port) = value
        .strip_prefix("http://127.0.0.1:")
        .and_then(|value| value.parse::<u16>().ok())
    else {
        return Err(BrowserDriverError::Denied);
    };
    if matches!(port, 0 | 80) {
        return Err(BrowserDriverError::Denied);
    }
    if !installation.capabilities.restrictive_egress
        || installation.capabilities.private_ca_trust
        || installation.capabilities.client_identities
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

async fn start(
    resource: &Arc<Mutex<Resources>>,
    enrollment: &BrowserBridgeEnrollment,
    startup: &Startup<'_>,
) -> Result<Arc<BrowserBridgeDriver>, BrowserDriverError> {
    let mut resource = resource.lock().await;
    startup.check()?;
    resource.installation.revalidate()?;
    startup.check()?;
    prepare_profile(&resource.installation.allocation)?;
    let profile = resource.installation.allocation.join("profile");
    let home = profile.join("home");
    let environment = [
        format!("HOME={}", home.display()),
        format!("XDG_CONFIG_HOME={}", home.join(".config").display()),
        format!("XDG_DATA_HOME={}", home.join(".local/share").display()),
        format!("TMPDIR={}", profile.join("tmp").display()),
        format!("MAC_CHROMIUM_TMPDIR={}", profile.join("tmp").display()),
        "PATH=/usr/bin:/bin".into(),
        "LANG=en_US.UTF-8".into(),
        "MallocNanoZone=0".into(),
    ]
    .map(OsString::from);
    #[cfg(feature = "macos-browser-tls-development")]
    let tls_ready = if resource.installation.tls_config.is_some() {
        resource.tls = Some(super::tls::Owner::new(
            &resource.installation,
            &resource.request,
        )?);
        Some(
            resource
                .tls
                .as_mut()
                .ok_or(BrowserDriverError::OutcomeUnknown)?
                .start(&environment, startup)
                .await?,
        )
    } else {
        None
    };
    if resource.installation.tls_config.is_none() {
        startup.check()?;
        resource.egress = Some(
            BrowserEgressLease::start_at_port_with_public_web(
                resource.request.session_id.clone(),
                resource.request.options.allowed_origins.clone(),
                resource.installation.egress,
                resource.installation.proxy_port,
                resource.request.options.public_web,
            )
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?,
        );
    }
    startup.check()?;
    let streams = spawn(
        &mut resource,
        enrollment.instance_nonce,
        &environment,
        startup,
    )?;
    for (index, output) in streams.output.into_iter().enumerate() {
        let name = match index {
            0 => "native-stdout.log".into(),
            1 => "native-stderr.log".into(),
            _ => format!("native-output-{index}.log"),
        };
        let capture = diagnostic_capture(&resource.installation.allocation, &name)?;
        resource.drains.push(tokio::task::spawn_blocking(move || {
            drain_diagnostic(output, capture)
        }));
    }
    for stream in [
        &streams.bootstrap,
        &streams.data,
        &streams.control,
        &streams.presentation,
    ] {
        stream
            .set_nonblocking(true)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    }
    let mut bootstrap = tokio::net::UnixStream::from_std(streams.bootstrap)
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    let data = tokio::net::UnixStream::from_std(streams.data)
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    let control = tokio::net::UnixStream::from_std(streams.control)
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    let presentation = tokio::net::UnixStream::from_std(streams.presentation)
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    receipt_phase(&mut bootstrap, 1).await?;
    if let Some(identity) = streams.main_identity {
        let body_identity = DarwinProcessIdentity::bind(identity.pid())
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        resource.peer = Some(
            DarwinAuditPeer::enroll(&body_identity, resource.installation.body_cdhash)
                .map_err(|_| BrowserDriverError::Denied)?,
        );
    }
    let (port, password) = proxy(
        &resource,
        #[cfg(feature = "macos-browser-tls-development")]
        tls_ready.as_ref(),
    )?;
    let bootstrap_value = Bootstrap {
        enrollment,
        profile_path: &profile,
        proxy: Proxy {
            address: "127.0.0.1",
            port,
            username: "colossus",
            password,
        },
        pki: None,
        broker_tls: {
            #[cfg(feature = "macos-browser-tls-development")]
            {
                tls_ready.as_ref().map(|ready| &ready.pin)
            }
            #[cfg(not(feature = "macos-browser-tls-development"))]
            {
                None
            }
        },
        presentation: Presentation {
            human_input: resource.request.run_id.is_none(),
        },
    };
    let bytes = Zeroizing::new(
        serde_json::to_vec(&bootstrap_value).map_err(|_| BrowserDriverError::Denied)?,
    );
    if bytes.is_empty() || bytes.len() > 32 * 1024 {
        return Err(BrowserDriverError::LimitExceeded);
    }
    let mut key = Zeroizing::new([0; 32]);
    getrandom::fill(key.as_mut()).map_err(|_| BrowserDriverError::Unavailable)?;
    startup.check()?;
    tokio::time::timeout(Duration::from_secs(5), async {
        startup.check_io()?;
        bootstrap.write_all(key.as_ref()).await?;
        startup.check_io()?;
        bootstrap
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await?;
        startup.check_io()?;
        bootstrap.write_all(&bytes).await?;
        bootstrap.shutdown().await
    })
    .await
    .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    for phase in 2..=7 {
        receipt_phase(&mut bootstrap, phase).await?;
    }
    let mut extra = [0];
    if tokio::time::timeout(Duration::from_secs(2), bootstrap.read(&mut extra))
        .await
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        != 0
    {
        return Err(BrowserDriverError::Denied);
    }
    let digest: [u8; 32] =
        Sha256::digest(serde_json::to_vec(enrollment).map_err(|_| BrowserDriverError::Denied)?)
            .into();
    let key = BrowserBridgeKey::from_bootstrap(key);
    let presentation_key = key.derive_presentation_key();
    let (data_reader, data_writer) = data.into_split();
    let (control_reader, control_writer) = control.into_split();
    let (presentation_reader, presentation_writer) = presentation.into_split();
    let (bridge, presenter) = tokio::try_join!(
        BrowserBridgeDriver::connect(
            InheritedBrowserChannel::new(data_reader, data_writer),
            InheritedBrowserChannel::new(control_reader, control_writer),
            enrollment.clone(),
            key
        ),
        async {
            PresentationClient::connect(
                PresentationChannel::new(presentation_reader, presentation_writer),
                presentation_key,
                digest,
            )
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)
        }
    )?;
    let bridge = Arc::new(bridge);
    resource.bridge = Some(Arc::clone(&bridge));
    resource.presentation = Some((presenter, digest));
    Ok(bridge)
}

struct NativeStreams {
    bootstrap: std::os::unix::net::UnixStream,
    data: std::os::unix::net::UnixStream,
    control: std::os::unix::net::UnixStream,
    presentation: std::os::unix::net::UnixStream,
    output: Vec<std::fs::File>,
    main_identity: Option<DarwinProcessIdentity>,
}

fn spawn(
    resource: &mut Resources,
    nonce: [u8; 16],
    environment: &[OsString],
    startup: &Startup<'_>,
) -> Result<NativeStreams, BrowserDriverError> {
    startup.check()?;
    if let Some(owned) = &resource.installation.owned_process {
        let owner = owned.owner(
            &resource.installation.executable,
            resource.installation.front_cdhash,
            nonce,
            resource.installation.policy_sha256,
            environment,
        )?;
        // Retain before start: failure at any native creation transition must
        // leave the exact process obligations with the session resource.
        resource.process_owner = Some(owner);
        resource.spawn_attempted = true;
        let channels = resource
            .process_owner
            .as_mut()
            .ok_or(BrowserDriverError::OutcomeUnknown)?
            .start_with_admission(|| startup.check_io())
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        return Ok(NativeStreams {
            bootstrap: channels.bootstrap,
            data: channels.data,
            control: channels.control,
            presentation: channels.presentation,
            output: channels.output,
            main_identity: None,
        });
    }
    let arguments = ["3", "4", "5", "6"].map(OsString::from);
    resource.spawn_attempted = true;
    let spawned = spawn_suspended_private_channels(
        &resource.installation.executable,
        &arguments,
        environment,
    )
    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    let pid = spawned.child.pid();
    resource.child = Some(spawned.child);
    let identity =
        DarwinProcessIdentity::bind(pid).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    if identity.audit_session_id() != resource.installation.audit_session {
        return Err(BrowserDriverError::Denied);
    }
    resource.peer = Some(
        DarwinAuditPeer::enroll(&identity, resource.installation.front_cdhash)
            .map_err(|_| BrowserDriverError::Denied)?,
    );
    startup.check()?;
    resource
        .child
        .as_mut()
        .ok_or(BrowserDriverError::OutcomeUnknown)?
        .resume()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    startup.check()?;
    Ok(NativeStreams {
        bootstrap: spawned.bootstrap,
        data: spawned.data,
        control: spawned.control,
        presentation: spawned.presentation,
        output: vec![spawned.stdout, spawned.stderr],
        main_identity: Some(identity),
    })
}

fn prepare_profile(allocation: &std::path::Path) -> Result<(), BrowserDriverError> {
    let root =
        colossus_home::ConfinedRoot::bind(allocation).map_err(|_| BrowserDriverError::Denied)?;
    let directory = rustix::fs::open(
        allocation,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| BrowserDriverError::Denied)?;
    root.revalidate().map_err(|_| BrowserDriverError::Denied)?;
    // This diagnostic has no profile custody or reuse contract. Atomically
    // create a fresh directory and reject every preexisting file or directory.
    rustix::fs::mkdirat(&directory, "profile", rustix::fs::Mode::RWXU)
        .map_err(|_| BrowserDriverError::Denied)?;
    root.revalidate().map_err(|_| BrowserDriverError::Denied)?;
    for relative in [
        "profile",
        "profile/home",
        "profile/home/.config",
        "profile/home/.local",
        "profile/home/.local/share",
        "profile/tmp",
    ] {
        root.prepare_directory(std::path::Path::new(relative))
            .map_err(|_| BrowserDriverError::Denied)?;
    }
    Ok(())
}

fn diagnostic_capture(
    allocation: &std::path::Path,
    name: &str,
) -> Result<std::fs::File, BrowserDriverError> {
    let root =
        colossus_home::ConfinedRoot::bind(allocation).map_err(|_| BrowserDriverError::Denied)?;
    let file = root
        .open_file(std::path::Path::new(name))
        .map_err(|_| BrowserDriverError::Denied)?;
    if !file.was_created() {
        return Err(BrowserDriverError::Denied);
    }
    file.file()
        .try_clone()
        .map_err(|_| BrowserDriverError::Unavailable)
}

fn drain_diagnostic(mut source: std::fs::File, mut capture: std::fs::File) -> std::io::Result<u64> {
    use std::io::{Read as _, Write as _};
    let mut count = 0_u64;
    let mut block = [0_u8; 4096];
    loop {
        let read = source.read(&mut block)?;
        if read == 0 {
            capture.sync_all()?;
            return Ok(count);
        }
        if count + read as u64 > 64 * 1024 {
            return Err(std::io::Error::other(
                "native diagnostic output limit exceeded",
            ));
        }
        capture.write_all(&block[..read])?;
        count += read as u64;
    }
}

async fn receipt_phase(
    stream: &mut tokio::net::UnixStream,
    expected: u8,
) -> Result<(), BrowserDriverError> {
    let mut record = [0; 5];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut record))
        .await
        .map_err(|_| {
            eprintln!("Mac diagnostic startup: phase {expected} timed out");
            BrowserDriverError::OutcomeUnknown
        })?
        .map_err(|_| {
            eprintln!("Mac diagnostic startup: phase {expected} disconnected");
            BrowserDriverError::OutcomeUnknown
        })?;
    if record != [b'C', b'B', b'H', 1, expected] {
        eprintln!("Mac diagnostic startup: phase {expected} rejected");
        return Err(BrowserDriverError::Denied);
    }
    eprintln!("Mac diagnostic startup: phase {expected} completed");
    Ok(())
}

#[derive(Serialize)]
struct Bootstrap<'a> {
    enrollment: &'a BrowserBridgeEnrollment,
    profile_path: &'a std::path::Path,
    proxy: Proxy<'a>,
    pki: Option<()>,
    #[serde(skip_serializing_if = "Option::is_none")]
    broker_tls: Option<&'a BrokerTls>,
    presentation: Presentation,
}

#[derive(Serialize)]
pub(super) struct BrokerTls {
    pub origin: colossus_contracts::BrowserOrigin,
    pub leaf_der: Vec<u8>,
    pub leaf_sha256: [u8; 32],
    pub lifetime_ms: u32,
}

#[cfg(feature = "macos-browser-tls-development")]
fn proxy<'a>(
    resource: &'a Resources,
    tls: Option<&'a super::tls::HostBootstrap>,
) -> Result<(u16, &'a str), BrowserDriverError> {
    if let Some(tls) = tls {
        return Ok((tls.port, tls.credential.as_str()));
    }
    http_proxy(resource)
}
#[cfg(not(feature = "macos-browser-tls-development"))]
fn proxy(resource: &Resources) -> Result<(u16, &str), BrowserDriverError> {
    http_proxy(resource)
}
fn http_proxy(resource: &Resources) -> Result<(u16, &str), BrowserDriverError> {
    let egress = resource
        .egress
        .as_ref()
        .ok_or(BrowserDriverError::OutcomeUnknown)?;
    Ok((egress.address().port(), egress.credential().expose()))
}
#[derive(Serialize)]
struct Proxy<'a> {
    address: &'a str,
    port: u16,
    username: &'a str,
    password: &'a str,
}
#[derive(Serialize)]
struct Presentation {
    human_input: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    #[test]
    fn an_existing_profile_is_rejected_without_reusing_its_data() {
        let allocation = tempfile::tempdir().unwrap();
        let allocation = allocation.path().canonicalize().unwrap();
        std::fs::set_permissions(&allocation, std::fs::Permissions::from_mode(0o700)).unwrap();
        prepare_profile(&allocation).unwrap();
        std::fs::write(allocation.join("profile/old-session"), b"retained").unwrap();
        assert_eq!(
            prepare_profile(&allocation),
            Err(BrowserDriverError::Denied)
        );
        assert_eq!(
            std::fs::read(allocation.join("profile/old-session")).unwrap(),
            b"retained"
        );
    }

    #[test]
    fn profile_symlink_is_rejected_before_changing_external_permissions() {
        let allocation = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let allocation = allocation.path().canonicalize().unwrap();
        std::fs::set_permissions(&allocation, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(outside.path(), std::fs::Permissions::from_mode(0o775)).unwrap();
        symlink(outside.path(), allocation.join("profile")).unwrap();
        assert_eq!(
            prepare_profile(&allocation),
            Err(BrowserDriverError::Denied)
        );
        assert_eq!(
            std::fs::metadata(outside.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o775
        );
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
