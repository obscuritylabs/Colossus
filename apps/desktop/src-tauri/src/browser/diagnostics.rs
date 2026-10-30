//! Categorical, bounded observations; no request/response headers or cookie values.

use colossus_native_browser::BrowserEvent;
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
#[allow(clippy::struct_excessive_bools)] // Independent observation facts, not lifecycle/control state.
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserDiagnostics {
    pub(crate) origin: Option<String>,
    pub(crate) http_status: Option<u16>,
    pub(crate) redirects: u16,
    pub(crate) native_error: Option<i32>,
    pub(crate) client_certificate_requested: bool,
    pub(crate) tls_failed: bool,
    pub(crate) persistent: bool,
    pub(crate) custom_user_agent: bool,
    pub(crate) system_engine: bool,
}

impl BrowserDiagnostics {
    pub(super) fn set_origin(&mut self, address: &str) {
        self.origin = url::Url::parse(address)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
            .map(|url| url.origin().ascii_serialization());
    }

    pub(super) fn event(&mut self, event: &BrowserEvent) {
        match event {
            BrowserEvent::HttpStatus(status) => self.http_status = Some(*status),
            BrowserEvent::Redirected => self.redirects = self.redirects.saturating_add(1),
            BrowserEvent::NavigationFailure(code) => self.native_error = Some(*code),
            BrowserEvent::TlsFailed => self.tls_failed = true,
            BrowserEvent::AuthenticationRequired => self.client_certificate_requested = true,
            _ => {}
        }
    }

    pub(super) fn reset(&mut self) {
        self.http_status = None;
        self.redirects = 0;
        self.native_error = None;
        self.client_certificate_requested = false;
        self.tls_failed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_strip_credentials_paths_queries_and_fragments() {
        let mut value = BrowserDiagnostics::default();
        value.set_origin("https://user:password@example.com:8443/callback?code=secret#token");
        assert_eq!(value.origin.as_deref(), Some("https://example.com:8443"));
        let encoded = serde_json::to_string(&value).unwrap();
        for secret in ["password", "callback", "secret", "token"] {
            assert!(!encoded.contains(secret));
        }
        value.set_origin("file:///sensitive");
        assert!(value.origin.is_none());
    }
    #[test]
    fn redirects_are_bounded_and_reload_clears_stale_auth_results() {
        let mut value = BrowserDiagnostics {
            redirects: u16::MAX,
            ..Default::default()
        };
        value.event(&BrowserEvent::Redirected);
        value.event(&BrowserEvent::HttpStatus(401));
        value.event(&BrowserEvent::AuthenticationRequired);
        assert_eq!(value.redirects, u16::MAX);
        assert_eq!(value.http_status, Some(401));
        value.reset();
        assert!(value.http_status.is_none() && !value.client_certificate_requested);
    }
}
