use crate::{
    ControlPlaneClient, Failure, Result, StreamEvent, operations::valid_identity,
    stream_frames::Frames,
};
use futures::StreamExt;
use serde::Deserialize;
use std::time::Duration;
use tokio::{
    sync::watch,
    time::{Instant, timeout},
};
use url::Url;

/// One exact project task or conversation. The exclusive cursor belongs only to this scope.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchScope {
    /// Exact project identity.
    pub project: String,
    /// Exact conversation identity; mutually exclusive with task.
    pub thread: Option<String>,
    /// Exact task identity; mutually exclusive with thread.
    pub task: Option<String>,
    /// Exclusive server event cursor, or absent to replay from the beginning.
    pub after: Option<String>,
}
impl WatchScope {
    pub(crate) fn route(&self, origin: &Url) -> Result<(Url, u64)> {
        let (kind, scope) = match (&self.thread, &self.task) {
            (Some(thread), None) => ("threads", thread),
            (None, Some(task)) => ("tasks", task),
            _ => return Err(invalid_scope()),
        };
        if !valid_identity(&self.project) || !valid_identity(scope) {
            return Err(invalid_scope());
        }
        let after = self
            .after
            .as_deref()
            .unwrap_or("0")
            .parse::<u64>()
            .map_err(|_| invalid_scope())?;
        let mut url = origin.clone();
        url.path_segments_mut()
            .map_err(|()| invalid_scope())?
            .clear()
            .extend(["api", "projects", &self.project, kind, scope, "events"]);
        Ok((url, after))
    }
}
impl ControlPlaneClient {
    /// Subscribe to released events using native authentication. At most four watches are active;
    /// unary request slots remain separate. Delivery, reconnect, frame size and lifetime are bounded.
    /// A false callback return cancels the watch. Sign-out and target replacement fence all delivery.
    pub async fn watch<F>(&self, scope: WatchScope, listener: F) -> Result<u64>
    where
        F: Fn(StreamEvent) -> bool + Send + Sync + 'static,
    {
        let (id, generation, connection, cancellation, url, cursor) = {
            let mut selected = self.inner.selected.lock().await;
            if selected.watches.len() >= 4 {
                return Err(Failure::new("busy", "Too many activity streams are open."));
            }
            let connection = selected.connection.clone().ok_or_else(invalid_scope)?;
            let (url, cursor) = scope.route(&connection.origin)?;
            selected.next_watch = selected
                .next_watch
                .checked_add(1)
                .ok_or_else(invalid_scope)?;
            let id = selected.next_watch;
            let (cancel, cancellation) = watch::channel(false);
            selected.watches.insert(id, cancel);
            (
                id,
                selected.generation,
                connection,
                cancellation,
                url,
                cursor,
            )
        };
        let client = self.clone();
        tokio::spawn(async move {
            let mut subscription = Subscription {
                client,
                id,
                generation,
                connection,
                cancellation,
                url,
                cursor,
            };
            let result = subscription.run(&listener).await;
            if let Err(error) = result {
                subscription
                    .emit(
                        &listener,
                        StreamEvent::Error {
                            code: error.code,
                            message: error.message,
                        },
                    )
                    .await;
                if error.code == "session_expired" {
                    let mut selected = subscription.client.inner.selected.lock().await;
                    if selected.generation == generation {
                        selected.replace(
                            crate::transport::Connection::new(
                                subscription.connection.origin.clone(),
                            )
                            .ok(),
                        );
                    }
                }
            }
            subscription.client.unwatch(id).await;
        });
        Ok(id)
    }
    /// Cancel one renderer-owned subscription without affecting its accepted runtime work.
    pub async fn unwatch(&self, id: u64) {
        if let Some(cancellation) = self.inner.selected.lock().await.watches.remove(&id) {
            let _ = cancellation.send(true);
        }
    }
}
struct Subscription {
    client: ControlPlaneClient,
    id: u64,
    generation: u64,
    connection: crate::transport::Connection,
    cancellation: watch::Receiver<bool>,
    url: Url,
    cursor: u64,
}
impl Subscription {
    async fn emit<F: Fn(StreamEvent) -> bool>(&self, listener: &F, event: StreamEvent) -> bool {
        // The short synchronous delivery is serialized with cancellation and session replacement.
        let selected = self.client.inner.selected.lock().await;
        selected.generation == self.generation
            && selected.watches.contains_key(&self.id)
            && listener(event)
    }
    async fn run<F: Fn(StreamEvent) -> bool>(&mut self, listener: &F) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(15 * 60);
        let mut attempts = 0u32;
        let mut total_bytes = 0usize;
        let mut total_events = 0usize;
        loop {
            let connection = tokio::select! {
                _ = self.cancellation.changed() => return Ok(()),
                _ = tokio::time::sleep_until(deadline) => return Err(expired()),
                result = self.connection.stream(self.url.clone(), self.cursor) => result,
            };
            let response = match connection {
                Ok(response) => Some(response),
                Err(error) if matches!(error.code, "unavailable" | "busy") => None,
                Err(error) => return Err(error),
            };
            if let Some(response) = response {
                if !self
                    .emit(listener, StreamEvent::Connection { connected: true })
                    .await
                {
                    return Ok(());
                }
                let mut stream = response.bytes_stream();
                let mut frames = Frames::default();
                loop {
                    let chunk = tokio::select! {
                        _ = self.cancellation.changed() => return Ok(()),
                        _ = tokio::time::sleep_until(deadline) => return Err(expired()),
                        chunk = timeout(Duration::from_secs(45), stream.next()) => chunk,
                    };
                    let Ok(Some(Ok(bytes))) = chunk else {
                        break;
                    };
                    total_bytes = total_bytes.saturating_add(bytes.len());
                    if total_bytes > 32 * 1024 * 1024 {
                        return Err(expired());
                    }
                    let events = frames.push(&bytes, &mut self.cursor)?;
                    for event in events {
                        total_events += 1;
                        if total_events > 4096 {
                            return Err(expired());
                        }
                        attempts = 0;
                        if !self.emit(listener, event).await {
                            return Ok(());
                        }
                    }
                }
            }
            if !self
                .emit(listener, StreamEvent::Connection { connected: false })
                .await
            {
                return Ok(());
            }
            attempts += 1;
            if attempts > 5 {
                return Err(Failure::new(
                    "unavailable",
                    "Activity disconnected. Refresh to reconnect.",
                ));
            }
            tokio::select! {
                _ = self.cancellation.changed() => return Ok(()),
                _ = tokio::time::sleep_until(deadline) => return Err(expired()),
                () = tokio::time::sleep(Duration::from_millis(250 * 2u64.pow(attempts - 1))) => {},
            }
        }
    }
}
fn invalid_scope() -> Failure {
    Failure::new(
        "invalid_argument",
        "Choose one project conversation or task to watch.",
    )
}
fn expired() -> Failure {
    Failure::new(
        "stream_limit",
        "Activity reached its connection limit. Refresh to reconnect.",
    )
}
