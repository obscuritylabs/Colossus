use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct WatchLifetime(Arc<AtomicUsize>);
impl Drop for WatchLifetime {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn quiescence_joins_every_watch_and_releases_its_outbound_sender() {
    let (sender, mut stream) = crate::outbound::channel();
    let alive = Arc::new(AtomicUsize::new(3));
    let mut watchers = JoinSet::new();
    for _ in 0..3 {
        let sender = sender.clone();
        let lifetime = WatchLifetime(Arc::clone(&alive));
        let (started, ready) = tokio::sync::oneshot::channel();
        watchers.spawn(async move {
            let _lifetime = lifetime;
            let _sender = sender;
            started.send(()).unwrap();
            std::future::pending::<Result<(), Status>>().await
        });
        ready.await.unwrap();
    }
    drop(sender);
    quiesce_watchers(&mut watchers).await;
    assert_eq!(
        alive.load(Ordering::SeqCst),
        0,
        "all upload futures dropped before granting new sharing"
    );
    assert!(watchers.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .is_none(),
        "no cancelled watch retains an outbound stream sender"
    );
}

#[tokio::test]
async fn transport_error_cleanup_preserves_accepted_runtime_work() {
    let (release, accepted_work) = tokio::sync::oneshot::channel();
    let accepted = tokio::spawn(async move { accepted_work.await.unwrap() });
    let mut watchers = JoinSet::new();
    watchers.spawn(async { Err(Status::unavailable("synthetic transport failure")) });
    watchers.spawn(std::future::pending::<Result<(), Status>>());
    let failure = watchers.join_next().await.unwrap().unwrap().unwrap_err();
    assert_eq!(failure.code(), tonic::Code::Unavailable);
    quiesce_watchers(&mut watchers).await;
    assert!(
        !accepted.is_finished(),
        "watch cleanup cannot cancel independently accepted local work"
    );
    release.send("retained result").unwrap();
    assert_eq!(accepted.await.unwrap(), "retained result");
}
