use super::*;
use colossus_browser_presentation::{PresentationChannel, serve_presentation};

pub(super) async fn handle(
    stream: platform::Stream,
    service: Arc<NativeBrowserService>,
) -> Result<(), WorkerError> {
    let digest = service
        .enrollment
        .digest()
        .map_err(|_| WorkerError::Protocol("native enrollment invalid".into()))?;
    let mut channel = server_handshake(stream, &service.authentication, digest)
        .await
        .map_err(|_| WorkerError::Protocol("native admission authentication failed".into()))?;
    let request = channel
        .receive_request()
        .await
        .map_err(|_| WorkerError::Protocol("native admission request rejected".into()))?;
    if !service.authorized() {
        channel
            .reply(&NativeBrowserReply::Denied)
            .await
            .map_err(admission_error)?;
        return Ok(());
    }
    if let NativeBrowserRequest::Open(request) = request {
        if !service.available()
            || service
                .admitted
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < MAX_LIFETIME_ADMISSIONS).then(|| count + 1)
                })
                .is_err()
        {
            channel
                .reply(&NativeBrowserReply::Unavailable)
                .await
                .map_err(admission_error)?;
            return Ok(());
        }
        let authority = service
            .authority
            .as_ref()
            .ok_or(WorkerError::BrowserCleanupUnknown)?;
        let session = match service
            .runtime
            .open_native_browser(
                authority,
                colossus_runtime::NativeBrowserOpenRequest {
                    conversation_id: request.conversation_id,
                    url: request.url,
                    width: request.width,
                    height: request.height,
                    scale_milli: request.scale_milli,
                    viewport_generation: request.viewport_generation,
                    lease_ms: request.lease_ms,
                },
            )
            .await
        {
            Ok(session) => Arc::new(session),
            Err(error) => {
                if error != colossus_runtime::RuntimeNativeBrowserError::OutcomeUnknown {
                    service.admitted.fetch_sub(1, Ordering::AcqRel);
                }
                channel
                    .reply(&native_error(error))
                    .await
                    .map_err(admission_error)?;
                return Ok(());
            }
        };
        service
            .sessions
            .lock()
            .map_err(|_| WorkerError::BrowserCleanupUnknown)?
            .insert(session.session_id().clone(), Arc::clone(&session));
        let _intent =
            lifecycle::DetachOnDrop::new(Arc::clone(&service), session.session_id().clone());
        if !service.authorized() {
            return Err(WorkerError::BrowserCleanupUnknown);
        }
        let configure = session
            .configure()
            .map_err(|_| WorkerError::BrowserCleanupUnknown)?;
        let enrollment_digest = session.enrollment_digest();
        channel
            .reply(&NativeBrowserReply::Opened {
                conversation_id: session.conversation_id().to_owned(),
                configure,
                enrollment_digest,
            })
            .await
            .map_err(admission_error)?;
        let (stream, key) = channel.into_presentation();
        let (reader, writer) = tokio::io::split(stream);
        let relay = Arc::new(relay::NativeRelay::new(Arc::clone(&service), session));
        let _ = serve_presentation(
            PresentationChannel::new(reader, writer),
            key,
            enrollment_digest,
            relay,
        )
        .await;
        return Ok(());
    }
    let reply = match request {
        NativeBrowserRequest::Probe => {
            if service.available() {
                NativeBrowserReply::Available
            } else {
                NativeBrowserReply::Unavailable
            }
        }
        NativeBrowserRequest::Close { session } => match service.finish(&session, true).await {
            Ok(()) => NativeBrowserReply::Closed,
            Err(_) => NativeBrowserReply::OutcomeUnknown,
        },
        NativeBrowserRequest::Detach { session } => match service.finish(&session, false).await {
            Ok(()) => NativeBrowserReply::Detached,
            Err(_) => NativeBrowserReply::OutcomeUnknown,
        },
        NativeBrowserRequest::Handoff {
            session,
            run_id,
            lease_ms,
        } => {
            if service.tracked(&session).is_none() {
                NativeBrowserReply::Denied
            } else if let Some(authority) = &service.authority {
                match service
                    .runtime
                    .handoff_native_browser(authority, &session, &run_id, lease_ms)
                    .await
                {
                    Ok(granted) if service.authorized() => NativeBrowserReply::Granted {
                        lease: granted.lease().clone(),
                        configure: granted.configure().clone(),
                        enrollment_digest: granted.enrollment_digest(),
                    },
                    Err(error) => native_error(error),
                    _ => NativeBrowserReply::OutcomeUnknown,
                }
            } else {
                NativeBrowserReply::Unavailable
            }
        }
        NativeBrowserRequest::Open(_) => unreachable!("Open handled before reply-only operations"),
    };
    if !service.authorized() {
        return Err(WorkerError::BrowserCleanupUnknown);
    }
    channel.reply(&reply).await.map_err(admission_error)
}
fn native_error(error: colossus_runtime::RuntimeNativeBrowserError) -> NativeBrowserReply {
    match error {
        colossus_runtime::RuntimeNativeBrowserError::Unavailable => NativeBrowserReply::Unavailable,
        colossus_runtime::RuntimeNativeBrowserError::OutcomeUnknown => {
            NativeBrowserReply::OutcomeUnknown
        }
        _ => NativeBrowserReply::Denied,
    }
}
fn admission_error(_: colossus_browser_presentation::PresentationError) -> WorkerError {
    WorkerError::Protocol("native admission exchange outcome unknown".into())
}
