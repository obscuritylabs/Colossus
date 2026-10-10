mod bootstrap;
use super::{
    WindowsBrowserPresentation,
    installation::Installation,
    lifecycle::{Driver, Resources},
    owner,
};
use crate::BrowserEgressLease;
use bootstrap::{Bootstrap, Presentation, Proxy, receipt};
use colossus_browser_bridge::{
    BrowserBridgeDriver, BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel,
};
use colossus_browser_presentation::{PresentationChannel, PresentationClient};
use colossus_contracts::{BrowserMode, BrowserProfileSelection, BrowserSessionId};
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use colossus_windows_native::PrivateDirectoryCreation;
use colossus_windows_process::{PrivatePipe, PrivateReader};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{io::AsyncWriteExt as _, sync::Mutex};
use zeroize::Zeroizing;

pub(super) async fn owned(
    installation: Arc<Installation>,
    sessions: Arc<Mutex<BTreeMap<BrowserSessionId, Arc<Mutex<Resources>>>>>,
    stopped: Arc<AtomicBool>,
    request: BrowserDriverOpenRequest,
    control: BrowserDriverControl,
) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
    if control.is_cancelled() || stopped.load(Ordering::Acquire) {
        return Err(BrowserDriverError::Cancelled);
    }
    if request.options.mode != BrowserMode::Embedded {
        return Err(BrowserDriverError::Unsupported);
    }
    if !matches!(request.options.profile, BrowserProfileSelection::Temporary) {
        return Err(BrowserDriverError::Unsupported);
    }
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|_| BrowserDriverError::Unavailable)?;
    if nonce == [0; 16] {
        return Err(BrowserDriverError::Unavailable);
    }
    let enrollment = BrowserBridgeEnrollment {
        binding: request.binding.clone(),
        mode: request.options.mode,
        profile: request.options.profile.clone(),
        allowed_origins: request.options.allowed_origins.clone(),
        instance_nonce: nonce,
        component_digest: installation.digest,
        cancellation_closes_context: true,
        capabilities: installation.capabilities.clone(),
    };
    enrollment.validate()?;
    let mut entries = sessions.lock().await;
    if control.is_cancelled() || stopped.load(Ordering::Acquire) {
        return Err(BrowserDriverError::Cancelled);
    }
    if entries.contains_key(&request.session_id) {
        return Err(BrowserDriverError::Denied);
    }
    entries.retain(|_, resource| {
        resource
            .try_lock()
            .map_or(true, |resource| !resource.closed)
    });
    if entries.len() >= usize::from(installation.capabilities.limits.max_sessions) {
        return Err(BrowserDriverError::LimitExceeded);
    }
    installation.revalidate()?;
    let directory = installation
        .state
        .canonical_path()
        .join(format!("session-{}", hex::encode(nonce)));
    let creation =
        PrivateDirectoryCreation::create(&directory).map_err(|_| BrowserDriverError::Denied)?;
    let profile = creation.path().join("profile");
    let fenced = Arc::new(AtomicBool::new(false));
    let resources = Arc::new(Mutex::new(Resources {
        request: request.clone(),
        creation,
        directory: None,
        profile,
        quarantine: None,
        owner: None,
        egress: None,
        bridge: None,
        presentation: None,
        presenter: None,
        io: Vec::new(),
        drains: Vec::new(),
        fenced: Arc::clone(&fenced),
        closed: false,
        cef_acknowledged: false,
        receipt: None,
    }));
    // Registration precedes proxy/process/thread allocation and the first await.
    entries.insert(request.session_id.clone(), Arc::clone(&resources));
    drop(entries);
    let result = start(&installation, &resources, &enrollment, &request, nonce).await;
    match result {
        Ok(bridge) if !control.is_cancelled() && !stopped.load(Ordering::Acquire) => {
            Ok(Arc::new(Driver {
                bridge,
                resources,
                request,
                fenced,
            }))
        }
        result => {
            resources.lock().await.cleanup().await?;
            Err(result.err().unwrap_or(BrowserDriverError::Cancelled))
        }
    }
}
async fn start(
    installation: &Arc<Installation>,
    resource: &Arc<Mutex<Resources>>,
    enrollment: &BrowserBridgeEnrollment,
    request: &BrowserDriverOpenRequest,
    nonce: [u8; 16],
) -> Result<Arc<BrowserBridgeDriver>, BrowserDriverError> {
    let mut resource = resource.lock().await;
    resource.directory = Some(
        resource
            .creation
            .bind()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
    );
    resource.egress = Some(
        BrowserEgressLease::start(
            request.session_id.clone(),
            enrollment.allowed_origins.clone(),
            installation.limits.egress,
        )
        .await
        .map_err(|_| BrowserDriverError::Unavailable)?,
    );
    let egress = resource
        .egress
        .as_ref()
        .ok_or(BrowserDriverError::OutcomeUnknown)?;
    let proxy_port = egress.address().port();
    let (owner, startup) = owner::Owner::start(owner::Request {
        installation: Arc::clone(installation),
        profile: resource.profile.clone(),
        proxy_port,
        nonce,
    })?;
    resource.owner = Some(owner);
    let child = tokio::time::timeout(Duration::from_secs(10), startup)
        .await
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        .map_err(|_| BrowserDriverError::OutcomeUnknown)??;
    let mut pipes = Vec::new();
    for channel in child.channels {
        let pipe = match PrivatePipe::from_files(channel.reader, channel.writer) {
            Ok(pipe) => pipe,
            Err(error) => {
                if let Some(lease) = error.lease {
                    resource.io.push(lease);
                }
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        };
        resource.io.push(pipe.lease.clone());
        pipes.push(pipe);
    }
    for file in [child.stdout, child.stderr].into_iter().flatten() {
        let (mut reader, lease) =
            PrivateReader::from_file(file).map_err(|_| BrowserDriverError::Unavailable)?;
        resource.io.push(lease);
        resource.drains.push(tokio::spawn(async move {
            tokio::io::copy(&mut reader, &mut tokio::io::sink()).await
        }));
    }
    let mut pipes = pipes.into_iter();
    let mut bootstrap = pipes.next().ok_or(BrowserDriverError::Denied)?;
    let data = pipes.next().ok_or(BrowserDriverError::Denied)?;
    let control = pipes.next().ok_or(BrowserDriverError::Denied)?;
    let presentation = pipes.next().ok_or(BrowserDriverError::Denied)?;
    if pipes.next().is_some() {
        return Err(BrowserDriverError::Denied);
    }
    let egress = resource
        .egress
        .as_ref()
        .ok_or(BrowserDriverError::OutcomeUnknown)?;
    let bytes = Zeroizing::new(
        serde_json::to_vec(&Bootstrap {
            enrollment,
            profile_path: &resource.profile,
            proxy: Proxy {
                address: "127.0.0.1",
                port: proxy_port,
                username: "colossus",
                password: egress.credential().expose(),
            },
            pki: None,
            presentation: Presentation {
                human_input: request.run_id.is_none(),
            },
        })
        .map_err(|_| BrowserDriverError::Denied)?,
    );
    if bytes.len() > 32 * 1024 {
        return Err(BrowserDriverError::LimitExceeded);
    }
    let digest =
        Sha256::digest(serde_json::to_vec(enrollment).map_err(|_| BrowserDriverError::Denied)?)
            .into();
    let mut key = Zeroizing::new([0; 32]);
    getrandom::fill(key.as_mut()).map_err(|_| BrowserDriverError::Unavailable)?;
    tokio::time::timeout(Duration::from_secs(5), async {
        bootstrap.writer.write_all(key.as_ref()).await?;
        bootstrap
            .writer
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await?;
        bootstrap.writer.write_all(&bytes).await?;
        bootstrap.writer.shutdown().await
    })
    .await
    .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    tokio::time::timeout(
        Duration::from_millis(u64::from(
            installation.capabilities.limits.navigation_timeout_ms,
        )),
        receipt(&mut bootstrap.reader),
    )
    .await
    .map_err(|_| BrowserDriverError::OutcomeUnknown)??;
    drop(bootstrap);
    let key = BrowserBridgeKey::from_bootstrap(key);
    let presentation_key = key.derive_presentation_key();
    let (bridge, client) = tokio::try_join!(
        BrowserBridgeDriver::connect(
            InheritedBrowserChannel::new(data.reader, data.writer),
            InheritedBrowserChannel::new(control.reader, control.writer),
            enrollment.clone(),
            key
        ),
        async {
            PresentationClient::connect(
                PresentationChannel::new(presentation.reader, presentation.writer),
                presentation_key,
                digest,
            )
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)
        }
    )?;
    let bridge = Arc::new(bridge);
    resource.bridge = Some(Arc::clone(&bridge));
    resource.presenter = Some(client.clone());
    resource.presentation = Some(WindowsBrowserPresentation {
        client,
        enrollment_digest: digest,
    });
    Ok(bridge)
}
