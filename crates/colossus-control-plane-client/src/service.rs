use crate::{
    Failure, Operation, Result,
    transport::{Connection, validate_origin},
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Semaphore, SemaphorePermit, watch};
use zeroize::Zeroizing;

pub(crate) struct Selection {
    pub generation: u64,
    pub connection: Option<Connection>,
    pub watches: HashMap<u64, watch::Sender<bool>>,
    pub next_watch: u64,
}
impl Selection {
    pub(crate) fn replace(&mut self, connection: Option<Connection>) {
        self.generation = self.generation.wrapping_add(1);
        self.connection = connection;
        for (_, cancellation) in self.watches.drain() {
            let _ = cancellation.send(true);
        }
    }
}
pub(crate) struct Inner {
    pub selected: Mutex<Selection>,
    permits: Semaphore,
}

/// Native session custody. Snapshots never cross IPC, and every response is fenced to its session generation.
#[derive(Clone)]
pub struct ControlPlaneClient {
    pub(crate) inner: Arc<Inner>,
}
impl Default for ControlPlaneClient {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                selected: Mutex::new(Selection {
                    generation: 0,
                    connection: None,
                    watches: HashMap::new(),
                    next_watch: 0,
                }),
                permits: Semaphore::new(8),
            }),
        }
    }
}
impl ControlPlaneClient {
    fn admit(&self) -> Result<SemaphorePermit<'_>> {
        self.inner
            .permits
            .try_acquire()
            .map_err(|_| Failure::new("busy", "Too many requests. Try again shortly."))
    }
    /// Select an HTTPS origin and discover sign-in methods. Replacing a server immediately cancels watches.
    /// Debug builds additionally accept literal loopback HTTP for isolated development fixtures.
    pub async fn connect(&self, origin: &str) -> Result<Value> {
        let _permit = self.admit()?;
        let origin = validate_origin(origin, cfg!(debug_assertions))?;
        let generation = {
            let mut selected = self.inner.selected.lock().await;
            selected.replace(None);
            selected.generation
        };
        let mut connection = Connection::new(origin)?;
        let configuration = connection.configuration().await?;
        if !configuration
            .get("local_enabled")
            .is_some_and(Value::is_boolean)
        {
            return Err(Failure::new(
                "unsupported_server",
                "This server does not provide a compatible Colossus sign-in service.",
            ));
        }
        let mut selected = self.inner.selected.lock().await;
        if selected.generation != generation {
            return Err(disconnected());
        }
        selected.connection = Some(connection);
        Ok(configuration)
    }
    /// Authenticate a local human account. Passwords and session cookies are never persisted or returned.
    pub async fn sign_in(&self, username: String, password: String) -> Result<Value> {
        let password = Zeroizing::new(password);
        let _permit = self.admit()?;
        if username.len() > 256 || password.len() > 1024 {
            return Err(Failure::new(
                "invalid_argument",
                "Check your username and password.",
            ));
        }
        let (generation, mut connection) = {
            let mut selected = self.inner.selected.lock().await;
            let origin = selected
                .connection
                .as_ref()
                .ok_or(disconnected())?
                .origin
                .clone();
            let connection = Connection::new(origin)?;
            selected.replace(Some(connection.clone()));
            (selected.generation, connection)
        };
        let identity = connection.login(username, password).await?;
        let mut selected = self.inner.selected.lock().await;
        if selected.generation != generation {
            return Err(disconnected());
        }
        selected.connection = Some(connection);
        Ok(identity)
    }
    /// Submit one closed operation. Mutations are attempted once; uncertain outcomes require reconciliation.
    /// Watches use separate admission slots so cancellation and interaction responses retain unary headroom.
    pub async fn request(&self, operation: Operation) -> Result<Value> {
        let _permit = self.admit()?;
        let (generation, mut connection) = self.snapshot().await?;
        let (method, url, body) = operation.route(&connection.origin)?;
        let result = connection.request(method, url, body, true).await;
        let mut selected = self.inner.selected.lock().await;
        if selected.generation != generation {
            return Err(disconnected());
        }
        if result
            .as_ref()
            .is_err_and(|error| error.code == "session_expired")
        {
            selected.replace(Some(Connection::new(connection.origin)?));
        }
        result
    }
    pub(crate) async fn snapshot(&self) -> Result<(u64, Connection)> {
        let selected = self.inner.selected.lock().await;
        Ok((
            selected.generation,
            selected.connection.clone().ok_or(disconnected())?,
        ))
    }
    /// Attempt remote logout and immediately discard local custody and every active watch.
    pub async fn sign_out(&self) -> Result<()> {
        let connection = {
            let mut selected = self.inner.selected.lock().await;
            let connection = selected.connection.take();
            selected.replace(None);
            connection
        };
        if let Some(mut connection) = connection {
            connection.logout().await;
        }
        Ok(())
    }
    /// Construct a browser destination on the selected server. No credentials are included.
    pub async fn browser_url(&self, project: Option<&str>, task: Option<&str>) -> Result<String> {
        let _permit = self.admit()?;
        let (_, connection) = self.snapshot().await?;
        let mut url = connection.origin;
        if let Some(project) = project {
            let mut segments = vec!["projects", project];
            if let Some(task) = task {
                segments.extend(["tasks", task]);
            }
            if segments
                .iter()
                .any(|s| !crate::operations::valid_identity(s))
            {
                return Err(Failure::new(
                    "invalid_argument",
                    "Invalid resource address.",
                ));
            }
            if let Ok(mut path) = url.path_segments_mut() {
                path.clear().extend(segments);
            }
        } else if task.is_some() {
            return Err(Failure::new(
                "invalid_argument",
                "Choose a project for this task.",
            ));
        }
        Ok(url.to_string())
    }
}
fn disconnected() -> Failure {
    Failure::new("disconnected", "Connect to your server first.")
}
