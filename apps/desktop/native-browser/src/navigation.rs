use std::{
    collections::HashSet,
    net::IpAddr,
    sync::{Arc, RwLock},
};

use url::Url;

use crate::BrowserError;

const MAX_ADDRESS_BYTES: usize = 8_192;

/// Normalize a user-entered address without selecting a search provider.
///
/// # Errors
/// Rejects non-web schemes, local app origins, userinfo, and malformed addresses.
pub fn parse_address(address: &str) -> Result<Url, BrowserError> {
    let address = address.trim();
    if address.is_empty()
        || address.len() > MAX_ADDRESS_BYTES
        || address.chars().any(char::is_control)
        || address.contains('\\')
    {
        return Err(BrowserError::InvalidAddress);
    }
    let normalized = if address.contains("://") {
        address.to_owned()
    } else if address.starts_with("localhost:")
        || address.starts_with("127.0.0.1:")
        || address.starts_with("[::1]:")
    {
        format!("http://{address}")
    } else if address.contains(':') || address.starts_with('/') {
        return Err(BrowserError::InvalidAddress);
    } else {
        format!("https://{address}")
    };
    let url = Url::parse(&normalized).map_err(|_| BrowserError::InvalidAddress)?;
    if !web_address_allowed(&url) {
        return Err(BrowserError::InvalidAddress);
    }
    Ok(url)
}

fn web_address_allowed(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.');
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.as_str().len() <= MAX_ADDRESS_BYTES
        && !matches!(
            host,
            "tauri.localhost"
                | "ipc.localhost"
                | "asset.localhost"
                | "colossus-terminal.localhost"
                | "colossus-approval.localhost"
        )
        && !(is_loopback(url) && url.port_or_known_default() == Some(1420))
}

fn is_loopback(url: &Url) -> bool {
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Shared per-tab navigation rules. Loopback origins require explicit address entry.
/// This controls navigation, not arbitrary remote subresource networking.
#[derive(Clone, Default)]
pub struct NavigationPolicy {
    loopback: Arc<RwLock<HashSet<String>>>,
}

impl NavigationPolicy {
    /// Authorize one explicitly entered web destination for this tab.
    ///
    /// # Errors
    /// Fails closed for reserved app addresses or unavailable policy state.
    pub fn authorize(&self, url: &Url) -> Result<(), BrowserError> {
        if !web_address_allowed(url) {
            return Err(BrowserError::InvalidAddress);
        }
        if is_loopback(url) {
            self.loopback
                .write()
                .map_err(|_| BrowserError::Closed)?
                .insert(url.origin().ascii_serialization());
        }
        Ok(())
    }

    /// Check native top-level/frame navigations, including redirects.
    #[must_use]
    pub fn allows(&self, address: &str) -> bool {
        if address == "about:blank" {
            return true;
        }
        let Ok(url) = Url::parse(address) else {
            return false;
        };
        web_address_allowed(&url)
            && (!is_loopback(&url)
                || self
                    .loopback
                    .read()
                    .is_ok_and(|origins| origins.contains(&url.origin().ascii_serialization())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_web_only_and_never_contain_credentials() {
        assert_eq!(
            parse_address("example.com/docs").unwrap().as_str(),
            "https://example.com/docs"
        );
        assert_eq!(
            parse_address("localhost:3000").unwrap().as_str(),
            "http://localhost:3000/"
        );
        for bad in [
            "",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,x",
            "tauri://localhost",
            "https://user:secret@example.com",
            "http://ipc.localhost",
            "http://tauri.localhost",
            "http://tauri.localhost./",
            "http://localhost.:1420/",
            "http://colossus-approval.localhost",
            "http://127.0.0.1:1420",
            "https://example.com\\@localhost",
            "https://example.com/\nx",
            "/workspace",
        ] {
            assert!(parse_address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn local_preview_authority_is_exact_and_not_shared_between_tabs() {
        let policy = NavigationPolicy::default();
        let local = parse_address("http://127.0.0.1:3000").unwrap();
        assert!(!policy.allows(local.as_str()));
        policy.authorize(&local).unwrap();
        assert!(policy.allows("http://127.0.0.1:3000/hmr"));
        assert!(!policy.allows("http://127.0.0.1:3001"));
        assert!(!NavigationPolicy::default().allows(local.as_str()));
        assert!(!policy.allows("http://127.0.0.1:1420"));
        assert!(policy.allows("https://example.com"));
    }
}
