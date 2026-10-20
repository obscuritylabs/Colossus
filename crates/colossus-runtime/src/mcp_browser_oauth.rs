//! Host-authorized browser OAuth for fixed standalone MCP connections.

use super::*;

impl Runtime {
    fn browser_oauth_executor(&self) -> Result<Arc<McpExecutor>, RuntimeError> {
        self._workspace_lease.identity().revalidate()?;
        // Keep one immutable executor across the human redirect. Ordinary run
        // catalogs are separate snapshots, and cannot own this host flow's lifetime.
        self.plugin_catalog
            .mcp_template
            .get()
            .cloned()
            .ok_or_else(|| RuntimeError::Config("MCP browser OAuth is unavailable".into()))
    }

    /// Begin human OAuth for one fixed standalone MCP declaration.
    ///
    /// Trusted host composition must authenticate and authorize the human, select the
    /// exact HTTPS callback, and bind the opaque flow to the project, user, session
    /// epoch and configuration revision. Neither binding nor callback is agent input.
    pub async fn mcp_oauth_browser_login_begin(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
        callback_url: &str,
    ) -> Result<McpOAuthBrowserLogin, RuntimeError> {
        self.browser_oauth_executor()?
            .oauth_browser_login_begin(server, flow_id, binding, callback_url)
            .await
            .map_err(Into::into)
    }

    /// Begin OAuth with an explicitly trusted callback transport policy.
    /// Local development loopback is a host deployment choice, never agent input.
    pub async fn mcp_oauth_browser_login_begin_with_policy(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
        callback_url: &str,
        callback_policy: McpOAuthBrowserCallbackPolicy,
    ) -> Result<McpOAuthBrowserLogin, RuntimeError> {
        self.browser_oauth_executor()?
            .oauth_browser_login_begin_with_policy(
                server,
                flow_id,
                binding,
                callback_url,
                callback_policy,
            )
            .await
            .map_err(Into::into)
    }

    /// Complete one caller-bound OAuth redirect against the same runtime instance.
    /// The host must recheck human authority and the original immutable binding.
    pub async fn mcp_oauth_browser_login_complete(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
        redirected_callback_url: &str,
    ) -> Result<McpOAuthStatus, RuntimeError> {
        self.browser_oauth_executor()?
            .oauth_browser_login_complete(server, flow_id, binding, redirected_callback_url)
            .await
            .map_err(Into::into)
    }

    /// Discard only the exact host-authorized flow; no credential is removed.
    pub async fn mcp_oauth_browser_login_cancel(
        &self,
        server: &str,
        flow_id: &str,
        binding: &str,
    ) -> Result<(), RuntimeError> {
        self.browser_oauth_executor()?
            .oauth_browser_login_cancel(server, flow_id, binding)
            .await
            .map_err(Into::into)
    }
}
