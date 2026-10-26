//! Preserve the Fleet RPC limit before decoding while relay RPCs use separate capacity.
use axum::http::{Request, Response};
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::{
    Status,
    body::Body,
    server::NamedService,
    transport::server::{TcpConnectInfo, TlsConnectInfo},
};
use tower::Service;

const PER_CONNECTION: usize = 2;
const MAX_ACTIVE_CONNECTIONS: usize = 2048;

#[derive(Clone)]
pub(crate) struct RuntimeConnectionLimit<S> {
    inner: S,
    slots: Arc<Mutex<HashMap<SocketAddr, Weak<Semaphore>>>>,
}
pub(crate) fn limit<S>(inner: S) -> RuntimeConnectionLimit<S> {
    RuntimeConnectionLimit {
        inner,
        slots: Arc::new(Mutex::new(HashMap::new())),
    }
}
impl<S: NamedService> NamedService for RuntimeConnectionLimit<S> {
    const NAME: &'static str = S::NAME;
}
impl<S> RuntimeConnectionLimit<S> {
    fn acquire(&self, peer: SocketAddr) -> Result<OwnedSemaphorePermit, Status> {
        let mut peers = self
            .slots
            .lock()
            .map_err(|_| Status::unavailable("Fleet request admission unavailable"))?;
        peers.retain(|_, slots| slots.strong_count() > 0);
        let slots = if let Some(slots) = peers.get(&peer).and_then(Weak::upgrade) {
            slots
        } else {
            if peers.len() >= MAX_ACTIVE_CONNECTIONS {
                return Err(Status::resource_exhausted("Fleet connection capacity"));
            }
            let slots = Arc::new(Semaphore::new(PER_CONNECTION));
            peers.insert(peer, Arc::downgrade(&slots));
            slots
        };
        slots
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("Fleet connection request capacity"))
    }
}
impl<S> Service<Request<Body>> for RuntimeConnectionLimit<S>
where
    S: Service<Request<Body>, Response = Response<Body>>,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }
    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let permit = peer(&request).and_then(|peer| self.acquire(peer));
        let permit = match permit {
            Ok(permit) => permit,
            Err(status) => return Box::pin(async move { Ok(status.into_http()) }),
        };
        // Acquire before the generated server can decode a protobuf request or Hello.
        let response = self.inner.call(request);
        Box::pin(async move {
            let response = response.await?;
            let (parts, body) = response.into_parts();
            Ok(Response::from_parts(
                parts,
                Body::new(HeldBody {
                    inner: body,
                    _permit: permit,
                }),
            ))
        })
    }
}
fn peer(request: &Request<Body>) -> Result<SocketAddr, Status> {
    let address = request
        .extensions()
        .get::<TlsConnectInfo<TcpConnectInfo>>()
        .and_then(|tls| tls.get_ref().remote_addr());
    #[cfg(test)]
    let address = address.or_else(|| {
        request
            .extensions()
            .get::<TcpConnectInfo>()
            .and_then(TcpConnectInfo::remote_addr)
    });
    address.ok_or_else(|| Status::unauthenticated("authenticated TLS connection required"))
}
struct HeldBody {
    inner: Body,
    _permit: OwnedSemaphorePermit,
}
impl HttpBody for HeldBody {
    type Data = <Body as HttpBody>::Data;
    type Error = Status;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(context)
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests;
