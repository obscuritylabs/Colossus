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
    if matches!(
        &request,
        NativeBrowserRequest::Open(_) | NativeBrowserRequest::Watch { .. }
    ) {
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
        let admitted = match &request {
            NativeBrowserRequest::Open(request) => {
                service
                    .runtime
                    .open_native_browser(
                        authority,
                        colossus_runtime::NativeBrowserOpenRequest {
                            conversation_id: request.conversation_id.clone(),
                            url: request.url.clone(),
                            width: request.width,
                            height: request.height,
                            scale_milli: request.scale_milli,
                            viewport_generation: request.viewport_generation,
                            lease_ms: request.lease_ms,
                        },
                    )
                    .await
            }
            NativeBrowserRequest::Watch {
                run_id,
                conversation_id,
            } => {
                service
                    .runtime
                    .watch_native_browser(authority, run_id, conversation_id)
                    .await
            }
            _ => unreachable!("only allocation or native viewer admission"),
        };
        let session = match admitted {
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
        {
            #[cfg(windows)]
            let _cleanup = service.cleanup.lock().await;
            service
                .sessions
                .lock()
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                .insert(session.session_id().clone(), Arc::clone(&session));
            #[cfg(windows)]
            service
                .detached
                .lock()
                .map_err(|_| WorkerError::BrowserCleanupUnknown)?
                .remove(session.session_id());
        }
        #[cfg(not(windows))]
        let _intent =
            lifecycle::DetachOnDrop::new(Arc::clone(&service), session.session_id().clone());
        #[cfg(windows)]
        let _intent =
            lifecycle::DetachOnDrop::new_owned(Arc::clone(&service), Arc::clone(&session));
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
        NativeBrowserRequest::PendingClientIdentity { policy } => match &service.authority {
            Some(authority) => match service
                .runtime
                .pending_native_browser_client_identity(authority, policy)
                .await
            {
                Ok(review) => NativeBrowserReply::ClientIdentity { review },
                Err(error) => native_error(error),
            },
            None => NativeBrowserReply::Unavailable,
        },
        NativeBrowserRequest::SelectClientIdentity {
            session,
            key,
            fingerprint_sha256,
        } => match &service.authority {
            Some(authority) => match service
                .runtime
                .select_native_browser_client_identity(authority, &session, key, fingerprint_sha256)
                .await
            {
                Ok(()) => NativeBrowserReply::ClientIdentitySelected,
                Err(error) => native_error(error),
            },
            None => NativeBrowserReply::Unavailable,
        },
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
        NativeBrowserRequest::Open(_) | NativeBrowserRequest::Watch { .. } => {
            unreachable!("admission handled before reply-only operations")
        }
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
