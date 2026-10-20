//! Host-bound browser OAuth flows; the host retains authenticated caller routing.

use super::*;
use std::sync::atomic::AtomicU64;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const FLOW_TTL: Duration = Duration::from_secs(300);
const MAX_PENDING_FLOWS: usize = 64;

/// Trusted host selection of the browser callback transport boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum McpOAuthBrowserCallbackPolicy {
    /// Production callbacks require HTTPS, including loopback destinations.
    #[default]
    HttpsOnly,
    /// An explicitly local development host may also use HTTP on an exact
    /// loopback IP literal or `localhost`. Other HTTP destinations remain denied.
    LocalDevelopmentLoopback,
}

/// Navigation metadata for one five-minute, single-use browser authorization.
///
/// The host must retain this executor instance until completion and bind the flow
/// to its authenticated user, project, session epoch and configuration revision.
/// Process restart invalidates pending flows. Neither tokens nor the host's
/// caller binding are released here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthBrowserLogin {
    /// Configured remote server name.
    pub server: String,
    /// Unique opaque identifier supplied by the trusted host.
    pub flow_id: String,
    /// Policy-validated authorization URL to open in the browser.
    pub authorization_url: String,
    /// Exact host-supplied callback without query or fragment.
    pub callback_url: String,
    /// Latest completion time, in Unix seconds.
    pub expires_at: u64,
}

#[derive(Default)]
pub(super) struct BrowserOAuthSessions {
    generation: AtomicU64,
    flows: tokio::sync::Mutex<BTreeMap<String, BrowserOAuthFlow>>,
}

struct BrowserOAuthFlow {
    server: String,
    binding: String,
    callback_url: String,
    callback_policy: McpOAuthBrowserCallbackPolicy,
    generation: u64,
    deadline: Instant,
    expires_at: u64,
    cancelled: bool,
    // A reserved or consumed entry keeps its ID until expiry, preventing overwrite
    // during discovery and reuse after completion or cancellation.
    pending: Option<PendingAuthorization>,
    state: Option<String>,
}

impl BrowserOAuthSessions {
    pub(super) fn advance_generation(&self) -> Result<u64, McpError> {
        self.generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map(|previous| previous + 1)
            .map_err(|_| invalid_flow())
    }

    pub(super) async fn cancel_server(&self, server: &str) {
        for flow in self.flows.lock().await.values_mut() {
            if flow.server == server {
                flow.pending = None;
                flow.state = None;
                flow.cancelled = true;
            }
        }
    }
}

impl McpExecutor {
    /// Begin browser authorization with an exact callback selected by the host.
    ///
    /// `flow_id` must be a unique opaque ASCII token of at most 128 bytes.
    /// `binding` is an opaque ASCII token of at most 256 bytes encoding the host's trusted
    /// user/project/session/configuration identity; renderer or model input must
    /// never choose it. At most 64 unexpired flows, including consumed IDs, are
    /// retained. Changed configuration snapshots invalidate older browser flows;
    /// equivalent read/run snapshots preserve them.
    /// The existing policy-bounded OAuth client and private credential store are
    /// reused; the HTTPS callback grants no additional network authority.
    pub async fn oauth_browser_login_begin(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
        callback_url: &str,
    ) -> Result<McpOAuthBrowserLogin, McpError> {
        self.oauth_browser_login_begin_with_policy(
            server,
            flow_id,
            binding,
            callback_url,
            McpOAuthBrowserCallbackPolicy::HttpsOnly,
        )
        .await
    }

    /// Begin a caller-bound browser flow under an explicit callback policy.
    ///
    /// Only a trusted host's local-development configuration may select
    /// `LocalDevelopmentLoopback`; renderer input must not choose this policy.
    /// All other bounds and authority checks match `oauth_browser_login_begin`.
    pub async fn oauth_browser_login_begin_with_policy(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
        callback_url: &str,
        callback_policy: McpOAuthBrowserCallbackPolicy,
    ) -> Result<McpOAuthBrowserLogin, McpError> {
        self.reserve_browser_flow(server, flow_id, binding, callback_url, callback_policy)
            .await?;
        let result = async {
            let (configured, _) = self.oauth_server(server)?;
            let manager = self
                .oauth_manager_with_redirect(
                    configured,
                    self.oauth_resource_authority,
                    &self.oauth_network_destinations,
                    &self.oauth_allowed_environment,
                    self.oauth_timeout_ms,
                    self.oauth_max_output_bytes,
                    callback_url,
                )
                .await?;
            self.activate_browser_flow(server, flow_id, binding, callback_url, manager)
                .await
        }
        .await;
        if result.is_err() {
            // Preserve the reserved ID; a failed start cannot race another start
            // with the same ID into replacing a human-reviewed authorization.
            let _ = self
                .oauth_browser_login_cancel(server, flow_id, binding)
                .await;
        }
        result
    }

    async fn reserve_browser_flow(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
        callback_url: &str,
        callback_policy: McpOAuthBrowserCallbackPolicy,
    ) -> Result<(), McpError> {
        validate_identity(flow_id, binding)?;
        fixed_callback(callback_url, callback_policy)?;
        self.oauth_server(server)?;
        if self
            .browser_oauth_sessions
            .generation
            .load(Ordering::Acquire)
            != self.browser_oauth_generation
        {
            return Err(invalid_flow());
        }
        let now = Instant::now();
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid_flow())?
            .as_secs()
            .checked_add(FLOW_TTL.as_secs())
            .ok_or_else(invalid_flow)?;
        let mut flows = self.browser_oauth_sessions.flows.lock().await;
        flows.retain(|_, flow| flow.deadline > now);
        if flows.len() >= MAX_PENDING_FLOWS || flows.contains_key(flow_id) {
            return Err(invalid_flow());
        }
        flows.insert(
            flow_id.into(),
            BrowserOAuthFlow {
                server: server.into(),
                binding: binding.into(),
                callback_url: callback_url.into(),
                callback_policy,
                generation: self.browser_oauth_generation,
                deadline: now + FLOW_TTL,
                expires_at,
                cancelled: false,
                pending: None,
                state: None,
            },
        );
        Ok(())
    }

    async fn activate_browser_flow(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
        callback_url: &str,
        manager: AuthorizationManager,
    ) -> Result<McpOAuthBrowserLogin, McpError> {
        let (configured, oauth) = self.oauth_server(server)?;
        let scopes = oauth.scopes.iter().map(String::as_str).collect::<Vec<_>>();
        let authorization_url = manager
            .get_authorization_url(&scopes)
            .await
            .map_err(safe_oauth_error)?;
        validate_oauth_authorization_url(
            &authorization_url,
            self.oauth_resource_authority,
            &self.oauth_network_destinations,
        )?;
        let authorization = url::Url::parse(&authorization_url).map_err(|_| invalid_flow())?;
        let state = query_value(&authorization, "state", 256)?;
        let mut flows = self.browser_oauth_sessions.flows.lock().await;
        let flow = flows.get_mut(flow_id).ok_or_else(invalid_flow)?;
        self.check_browser_flow(flow, server, binding)?;
        if flow.callback_url != callback_url {
            return Err(invalid_flow());
        }
        flow.state = Some(state);
        flow.pending = Some(PendingAuthorization {
            session: AuthorizationSession::for_scope_upgrade(
                manager,
                authorization_url.clone(),
                callback_url,
            ),
            endpoint: configured.url.clone(),
            oauth: oauth.clone(),
            provenance: configured.provenance.clone(),
        });
        Ok(McpOAuthBrowserLogin {
            server: server.into(),
            flow_id: flow_id.into(),
            authorization_url,
            callback_url: callback_url.into(),
            expires_at: flow.expires_at,
        })
    }

    /// Complete only the exact caller-bound browser flow, once, before expiry.
    ///
    /// The host must first authenticate its callback browser and recheck the
    /// binding. Callback origin/path and PKCE state are checked before consuming
    /// the pending session; rmcp then verifies PKCE and the authorization-server
    /// issuer. Changed configuration, logout, cancellation and restart invalidate
    /// pending flows. Errors contain no callback parameters or credentials.
    pub async fn oauth_browser_login_complete(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
        redirected_callback_url: &str,
    ) -> Result<McpOAuthStatus, McpError> {
        let _oauth_guard = self.lock_oauth(server, self.oauth_timeout_ms).await?;
        validate_identity(flow_id, binding)?;
        if redirected_callback_url.len() > 8192 {
            return Err(invalid_flow());
        }
        let callback = url::Url::parse(redirected_callback_url).map_err(|_| invalid_flow())?;
        let session = {
            let mut flows = self.browser_oauth_sessions.flows.lock().await;
            let flow = flows.get_mut(flow_id).ok_or_else(invalid_flow)?;
            self.check_browser_flow(flow, server, binding)?;
            validate_browser_callback(&callback, &flow.callback_url, flow.callback_policy)?;
            if flow.state.as_deref() != Some(query_value(&callback, "state", 256)?.as_str()) {
                return Err(invalid_flow());
            }
            query_value(&callback, "code", 4096)?;
            let (configured, oauth) = self.oauth_server(server)?;
            let pending = flow.pending.as_ref().ok_or_else(invalid_flow)?;
            if pending.endpoint != configured.url
                || pending.oauth != *oauth
                || pending.provenance != configured.provenance
            {
                return Err(invalid_flow());
            }
            flow.state = None;
            flow.cancelled = true;
            flow.pending.take().ok_or_else(invalid_flow)?
        };
        session
            .session
            .handle_callback_url(redirected_callback_url)
            .await
            .map_err(safe_oauth_error)?;
        self.oauth_status(server).await
    }

    /// Invalidate a matching caller-bound flow without modifying stored tokens.
    /// The consumed ID stays reserved until its original expiry.
    pub async fn oauth_browser_login_cancel(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
    ) -> Result<(), McpError> {
        validate_identity(flow_id, binding)?;
        let mut flows = self.browser_oauth_sessions.flows.lock().await;
        let flow = flows.get_mut(flow_id).ok_or_else(invalid_flow)?;
        if flow.server != server || flow.binding != binding {
            return Err(invalid_flow());
        }
        flow.pending = None;
        flow.state = None;
        flow.cancelled = true;
        Ok(())
    }

    fn check_browser_flow(
        &self,
        flow: &BrowserOAuthFlow,
        server: &str,
        binding: &str,
    ) -> Result<(), McpError> {
        if flow.cancelled
            || flow.server != server
            || flow.binding != binding
            || flow.deadline <= Instant::now()
            || flow.generation != self.browser_oauth_generation
            || self
                .browser_oauth_sessions
                .generation
                .load(Ordering::Acquire)
                != self.browser_oauth_generation
        {
            return Err(invalid_flow());
        }
        Ok(())
    }
}

fn validate_identity(flow_id: &str, binding: &str) -> Result<(), McpError> {
    let valid = |value: &str, maximum| {
        !value.is_empty()
            && value.len() <= maximum
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    if !valid(flow_id, 128) || !valid(binding, 256) {
        return Err(invalid_flow());
    }
    Ok(())
}

fn fixed_callback(
    value: &str,
    policy: McpOAuthBrowserCallbackPolicy,
) -> Result<url::Url, McpError> {
    if value.len() > 2048 {
        return Err(invalid_flow());
    }
    let url = url::Url::parse(value).map_err(|_| invalid_flow())?;
    let loopback = match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    if !(url.scheme() == "https"
        || (policy == McpOAuthBrowserCallbackPolicy::LocalDevelopmentLoopback
            && url.scheme() == "http"
            && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_flow());
    }
    Ok(url)
}

fn validate_browser_callback(
    callback: &url::Url,
    expected: &str,
    policy: McpOAuthBrowserCallbackPolicy,
) -> Result<(), McpError> {
    let expected = fixed_callback(expected, policy)?;
    let mut without_query = callback.clone();
    without_query.set_query(None);
    if without_query != expected {
        return Err(invalid_flow());
    }
    let mut names = BTreeSet::new();
    if callback
        .query_pairs()
        .any(|(name, value)| value.len() > 4096 || !names.insert(name.into_owned()))
    {
        return Err(invalid_flow());
    }
    Ok(())
}

fn query_value(url: &url::Url, name: &str, maximum: usize) -> Result<String, McpError> {
    let mut values = url.query_pairs().filter(|(key, _)| key == name);
    let (_, value) = values.next().ok_or_else(invalid_flow)?;
    if value.is_empty() || value.len() > maximum || values.next().is_some() {
        return Err(invalid_flow());
    }
    Ok(value.into_owned())
}

fn invalid_flow() -> McpError {
    McpError::OAuth("browser authorization flow is invalid, expired or no longer available".into())
}

#[cfg(test)]
mod tests;
