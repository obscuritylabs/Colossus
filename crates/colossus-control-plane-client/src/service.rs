use crate::{
    Failure, Operation, Result,
    transport::{Connection, validate_origin},
};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};
use zeroize::Zeroizing;

/// One selected server and an in-memory human session, serialized across target changes.
pub struct ControlPlaneClient {
    connection: Mutex<Option<Connection>>,
    permits: Semaphore,
}
impl Default for ControlPlaneClient {
    fn default() -> Self {
        Self {
            connection: Mutex::new(None),
            permits: Semaphore::new(8),
        }
    }
}
impl ControlPlaneClient {
    fn admit(&self) -> Result<SemaphorePermit<'_>> {
        self.permits
            .try_acquire()
            .map_err(|_| Failure::new("busy", "Too many requests. Try again shortly."))
    }
    /// Select an HTTPS origin and discover its sign-in methods. Replacing a server drops local custody.
    /// Debug builds additionally accept literal loopback HTTP for isolated development fixtures.
    pub async fn connect(&self, origin: &str) -> Result<Value> {
        let _permit = self.admit()?;
        let origin = validate_origin(origin, cfg!(debug_assertions))?;
        let mut selected = self.connection.lock().await;
        *selected = None;
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
        *selected = Some(connection);
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
        self.connection
            .lock()
            .await
            .as_mut()
            .ok_or(disconnected())?
            .login(username, password)
            .await
    }
    /// Submit one closed operation. Mutations are attempted once; uncertain outcomes require reconciliation.
    pub async fn request(&self, operation: Operation) -> Result<Value> {
        let _permit = self.admit()?;
        let mut selected = self.connection.lock().await;
        let connection = selected.as_mut().ok_or(disconnected())?;
        let (method, url, body) = operation.route(&connection.origin)?;
        connection.request(method, url, body, true).await
    }
    /// Attempt remote logout and always discard the local session, even when the server is unreachable.
    pub async fn sign_out(&self) -> Result<()> {
        // Logout cannot be rejected by the request admission limit. Remove the
        // selected session before waiting for best-effort remote revocation.
        let connection = self.connection.lock().await.take();
        if let Some(mut connection) = connection {
            connection.logout().await;
        }
        Ok(())
    }
    /// Construct a browser destination on the selected server. No credentials are included.
    pub async fn browser_url(&self, project: Option<&str>, task: Option<&str>) -> Result<String> {
        let _permit = self.admit()?;
        let selected = self.connection.lock().await;
        let connection = selected.as_ref().ok_or(disconnected())?;
        let mut url = connection.origin.clone();
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
