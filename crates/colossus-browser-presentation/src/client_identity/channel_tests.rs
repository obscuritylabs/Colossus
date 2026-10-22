//! Real authenticated transport for review before any visible lease exists.
use super::*;
use crate::{
    Frame, Lease, PresentationChannel, PresentationClient, PresentationCommand, PresentationHost,
    PresentationReply, serve_presentation,
};
use async_trait::async_trait;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use zeroize::Zeroizing;

struct Host {
    commands: Mutex<Vec<PresentationCommand>>,
    mode: u8,
    revoked: AtomicBool,
    blocked: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
fn challenge() -> ClientIdentityChallenge {
    ClientIdentityChallenge {
        key: ClientIdentityKey {
            tab: 7,
            generation: 2,
            request_id: 19,
            control_generation: 0,
        },
        origin: BrowserOrigin::parse("https://example.test:8443").unwrap(),
        candidates: vec![ClientIdentityCandidate {
            fingerprint_sha256: "a".repeat(64),
            label: "Public identity".into(),
        }],
        remaining_ms: 40_000,
    }
}
#[async_trait]
impl PresentationHost for Host {
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        self.commands.lock().unwrap().push(command.clone());
        if matches!(command, PresentationCommand::PendingClientIdentity { .. })
            && self.blocked.load(Ordering::Acquire)
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
        match command {
            PresentationCommand::PendingClientIdentity { .. } if self.mode == 1 => {
                Ok(PresentationReply::Ack)
            }
            PresentationCommand::PendingClientIdentity { .. } => {
                let mut challenge = challenge();
                if self.mode == 2 {
                    challenge.candidates[0].label = "\u{202e}forged".into();
                }
                Ok(PresentationReply::ClientIdentity {
                    challenge: Some(challenge),
                })
            }
            PresentationCommand::SelectClientIdentity { .. } if self.mode == 3 => {
                Err(PresentationError::OutcomeUnknown)
            }
            PresentationCommand::SelectClientIdentity { .. } => Ok(PresentationReply::Ack),
            _ => Err(PresentationError::Invalid),
        }
    }
    async fn frame(&self, _: Lease) -> Result<Option<Frame>, PresentationError> {
        Err(PresentationError::Hidden)
    }
    fn revoke_now(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    async fn revoke(&self) {
        self.revoke_now();
    }
}
async fn pair(
    mode: u8,
) -> (
    PresentationClient,
    Arc<Host>,
    tokio::task::JoinHandle<Result<(), PresentationError>>,
) {
    let host = Arc::new(Host {
        commands: Mutex::new(Vec::new()),
        mode,
        revoked: AtomicBool::new(false),
        blocked: AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let (client, server) = tokio::io::duplex(32 * 1024);
    let (client_read, client_write) = tokio::io::split(client);
    let (server_read, server_write) = tokio::io::split(server);
    let serving = tokio::spawn(serve_presentation(
        PresentationChannel::new(server_read, server_write),
        Zeroizing::new([1; 32]),
        [2; 32],
        host.clone(),
    ));
    let client = PresentationClient::connect(
        PresentationChannel::new(client_read, client_write),
        Zeroizing::new([1; 32]),
        [2; 32],
    )
    .await
    .unwrap();
    (client, host, serving)
}

#[tokio::test]
async fn authenticated_review_and_keyed_selection_work_before_configuration() {
    let (client, host, serving) = pair(0).await;
    for command in [
        PresentationCommand::PendingClientIdentity {
            policy: ClientIdentityPolicy::Review,
        },
        PresentationCommand::SelectClientIdentity {
            key: challenge().key,
            fingerprint_sha256: None,
        },
    ] {
        assert_eq!(
            client.command(command).await,
            Err(PresentationError::Invalid),
            "ordinary viewer command forwarding cannot select keys"
        );
    }
    assert!(host.commands.lock().unwrap().is_empty());
    let review = client
        .pending_client_identity(ClientIdentityPolicy::Review)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(review, challenge());
    client
        .select_client_identity(
            review.key,
            Some(review.candidates[0].fingerprint_sha256.clone()),
        )
        .await
        .unwrap();
    let commands = host.commands.lock().unwrap().clone();
    assert!(
        matches!(commands.as_slice(), [PresentationCommand::PendingClientIdentity { policy: ClientIdentityPolicy::Review }, PresentationCommand::SelectClientIdentity { key, fingerprint_sha256: Some(fingerprint) }] if *key == review.key && fingerprint == &review.candidates[0].fingerprint_sha256)
    );
    client.disconnect();
    assert!(serving.await.unwrap().is_err());
    assert!(host.revoked.load(Ordering::Acquire));
}

#[tokio::test]
async fn malformed_review_or_wrong_reply_closes_the_authenticated_channel() {
    for mode in [1, 2] {
        let (client, host, serving) = pair(mode).await;
        assert_eq!(
            client
                .pending_client_identity(ClientIdentityPolicy::Review)
                .await,
            Err(PresentationError::OutcomeUnknown)
        );
        assert_eq!(
            client.select_client_identity(challenge().key, None).await,
            Err(PresentationError::OutcomeUnknown)
        );
        assert!(serving.await.unwrap().is_err());
        assert_eq!(host.commands.lock().unwrap().len(), 1);
        assert!(host.revoked.load(Ordering::Acquire));
    }
}

#[tokio::test]
async fn uncertain_selection_is_not_replayed_on_the_native_channel() {
    let (client, host, serving) = pair(3).await;
    let review = client
        .pending_client_identity(ClientIdentityPolicy::Review)
        .await
        .unwrap()
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            client
                .select_client_identity(
                    review.key,
                    Some(review.candidates[0].fingerprint_sha256.clone())
                )
                .await,
            Err(PresentationError::OutcomeUnknown)
        );
    }
    assert!(serving.await.unwrap().is_err());
    assert_eq!(
        host.commands.lock().unwrap().len(),
        2,
        "one read and one submitted effect; no automatic mutation retry"
    );
    assert!(host.revoked.load(Ordering::Acquire));
}

#[tokio::test(start_paused = true)]
async fn abandoned_dispatched_review_drains_its_reply_before_the_next_choice() {
    let (client, host, serving) = pair(0).await;
    host.blocked.store(true, Ordering::Release);
    let reader = client.clone();
    let read = tokio::spawn(async move {
        reader
            .pending_client_identity(ClientIdentityPolicy::Review)
            .await
    });
    host.entered.notified().await;
    // Runtime's bounded lookup can expire after dispatch, before the host's
    // original 750ms channel deadline. Abandon only that metadata receiver.
    tokio::time::advance(std::time::Duration::from_millis(500)).await;
    read.abort();
    assert!(read.await.unwrap_err().is_cancelled());
    tokio::task::yield_now().await;
    let reader = client.clone();
    let next = tokio::spawn(async move {
        reader
            .pending_client_identity(ClientIdentityPolicy::Review)
            .await
    });
    tokio::task::yield_now().await;
    host.blocked.store(false, Ordering::Release);
    host.release.notify_one();
    let review = next.await.unwrap().unwrap().unwrap();
    assert_eq!(review, challenge());
    client
        .select_client_identity(
            review.key,
            Some(review.candidates[0].fingerprint_sha256.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        host.commands.lock().unwrap().len(),
        3,
        "abandoned read, fresh read, one keyed choice"
    );
    assert!(!host.revoked.load(Ordering::Acquire));
    client.disconnect();
    assert!(serving.await.unwrap().is_err());
}

#[tokio::test(start_paused = true)]
async fn abandoned_automatic_policy_still_closes_the_channel_without_replay() {
    let (client, host, serving) = pair(0).await;
    host.blocked.store(true, Ordering::Release);
    let owner = client.clone();
    let automatic = tokio::spawn(async move {
        owner
            .pending_client_identity(ClientIdentityPolicy::AutomaticIfUnique)
            .await
    });
    host.entered.notified().await;
    automatic.abort();
    assert!(automatic.await.unwrap_err().is_cancelled());
    tokio::task::yield_now().await;
    host.blocked.store(false, Ordering::Release);
    host.release.notify_one();
    assert!(serving.await.unwrap().is_err());
    assert!(host.revoked.load(Ordering::Acquire));
    for _ in 0..2 {
        assert_eq!(
            client
                .pending_client_identity(ClientIdentityPolicy::AutomaticIfUnique)
                .await,
            Err(PresentationError::OutcomeUnknown)
        );
    }
    assert_eq!(
        host.commands.lock().unwrap().len(),
        1,
        "abandoned automatic policy is sent once only"
    );
}
