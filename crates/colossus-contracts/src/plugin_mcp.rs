//! Canonical effect identities for runtime-bound plugin MCP servers.

/// Encode both validated name components without ambiguous dot concatenation.
///
/// The length prefixes keep `dev.example/server` distinct from `dev/example.server`.
/// This is an effect-action namespace, not the portable `plugin/server` identity.
#[must_use]
pub fn plugin_mcp_action_prefix(plugin: &str, server: &str) -> String {
    format!(
        "plugin.mcp.{}.{plugin}.{}.{server}",
        plugin.len(),
        server.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_components_cannot_share_effect_authority() {
        assert_eq!(
            plugin_mcp_action_prefix("dev.example", "server"),
            "plugin.mcp.11.dev.example.6.server"
        );
        assert_ne!(
            plugin_mcp_action_prefix("dev.example", "server"),
            plugin_mcp_action_prefix("dev", "example.server")
        );
    }
}
