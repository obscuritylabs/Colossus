use super::*;
use std::{
    convert::Infallible,
    future::{Ready, ready},
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Spy(Arc<AtomicUsize>);
impl NamedService for Spy {
    const NAME: &'static str = "colossus.cloud.v1alpha1.RuntimeConnection";
}
struct PendingBody;
impl HttpBody for PendingBody {
    type Data = <Body as HttpBody>::Data;
    type Error = Status;
    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Pending
    }
}
impl Service<Request<Body>> for Spy {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, _: Request<Body>) -> Self::Future {
        self.0.fetch_add(1, Ordering::SeqCst);
        ready(Ok(Response::new(Body::new(PendingBody))))
    }
}
fn request(port: u16) -> Request<Body> {
    let mut request = Request::new(Body::empty());
    request.extensions_mut().insert(TcpConnectInfo {
        local_addr: Some("127.0.0.1:8443".parse().unwrap()),
        remote_addr: Some(SocketAddr::from(([127, 0, 0, 1], port))),
    });
    request
}
#[tokio::test]
async fn fleet_limit_is_per_tcp_peer_before_delegate_and_retained_until_stream_drop() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = limit(Spy(calls.clone()));
    let first = service.call(request(1234)).await.unwrap();
    let second = service.clone().call(request(1234)).await.unwrap();
    let denied = service.call(request(1234)).await.unwrap();
    assert_eq!(denied.headers()["grpc-status"], "8");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "rejected request never reaches protobuf handler"
    );
    let different_peer = service.call(request(1235)).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "two is not a process-global Fleet limit"
    );
    drop(first);
    let resumed = service.call(request(1234)).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    drop((second, different_peer, resumed));
    service
        .slots
        .lock()
        .unwrap()
        .retain(|_, slots| slots.strong_count() > 0);
    assert!(service.slots.lock().unwrap().is_empty());
}
#[tokio::test]
async fn absent_transport_identity_is_rejected_before_delegate() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut service = limit(Spy(calls.clone()));
    let denied = service.call(Request::new(Body::empty())).await.unwrap();
    assert_eq!(denied.headers()["grpc-status"], "16");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
