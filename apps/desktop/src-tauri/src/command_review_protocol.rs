//! Fixed bundled approval document: no remote assets, navigation, or general IPC.

use tauri::{Runtime, UriSchemeContext, WebviewUrl, http};

pub(crate) const SCHEME: &str = "colossus-approval";
const REVIEW_CSP: &str = "default-src 'self'; script-src 'self'; connect-src ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; media-src 'none'; form-action 'none'";

pub(crate) fn window_url() -> WebviewUrl {
    #[cfg(debug_assertions)]
    {
        WebviewUrl::App("index.html?surface=command-approval".into())
    }
    #[cfg(not(debug_assertions))]
    WebviewUrl::CustomProtocol(
        format!("{SCHEME}://localhost/index.html?surface=command-approval")
            .parse()
            .expect("fixed approval URL"),
    )
}

pub(crate) fn navigation_allowed(url: &tauri::Url) -> bool {
    navigation_allowed_for_profile(url, cfg!(debug_assertions))
}

fn navigation_allowed_for_profile(url: &tauri::Url, debug: bool) -> bool {
    let release_origin = url.port().is_none()
        && ((url.scheme() == SCHEME && url.host_str() == Some("localhost"))
            || (url.scheme() == "https" && url.host_str() == Some("colossus-approval.localhost")));
    let debug_origin = debug
        && ((url.scheme() == "tauri"
            && url.host_str() == Some("localhost")
            && url.port().is_none())
            || (url.scheme() == "https"
                && url.host_str() == Some("tauri.localhost")
                && url.port().is_none())
            || (url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port() == Some(1420)));
    (release_origin || debug_origin)
        && matches!(url.path(), "/index.html" | "/")
        && url.query() == Some("surface=command-approval")
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

pub(crate) fn respond<R: Runtime>(
    context: &UriSchemeContext<'_, R>,
    request: &http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let valid = context.webview_label() == crate::command_review::WINDOW
        && request.method() == http::Method::GET
        && matches!(
            (
                request.uri().scheme_str(),
                request.uri().authority().map(http::uri::Authority::as_str)
            ),
            (Some(SCHEME), Some("localhost"))
                | (Some("https"), Some("colossus-approval.localhost"))
        );
    let path = request.uri().path();
    let document =
        path == "/index.html" && request.uri().query() == Some("surface=command-approval");
    let asset = path.strip_prefix("/assets/").is_some_and(|name| {
        name.len() <= 192
            && !name.contains("..")
            && !name.contains('/')
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            && std::path::Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "js" || extension == "css")
    }) && request.uri().query().is_none();
    let resolved = (valid && (document || asset))
        .then(|| {
            context
                .app_handle()
                .asset_resolver()
                .get_for_scheme(path.to_owned(), false)
        })
        .flatten();
    let (status, mime, body) = match resolved {
        Some(asset) if asset.bytes().len() <= 8 * 1024 * 1024 => (
            http::StatusCode::OK,
            asset.mime_type().to_owned(),
            asset.bytes,
        ),
        _ => (http::StatusCode::NOT_FOUND, "text/plain".into(), Vec::new()),
    };
    http::Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Cache-Control", "no-store")
        .header("Content-Security-Policy", REVIEW_CSP)
        .header("Cross-Origin-Opener-Policy", "same-origin")
        .header("X-Frame-Options", "DENY")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .body(body)
        .expect("fixed response headers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_fixed_review_document_can_navigate_or_invoke() {
        for url in [
            "colossus-approval://localhost/index.html?surface=command-approval",
            "https://colossus-approval.localhost/index.html?surface=command-approval",
        ] {
            assert!(navigation_allowed_for_profile(&url.parse().unwrap(), false));
        }
        for url in [
            "https://example.com/index.html?surface=command-approval",
            "colossus-approval://localhost/index.html?surface=terminal",
            "colossus-approval://user@localhost/index.html?surface=command-approval",
            "colossus-approval://localhost/index.html?surface=command-approval#spoof",
            "http://127.0.0.1:1420/index.html?surface=command-approval",
        ] {
            assert!(!navigation_allowed_for_profile(
                &url.parse().unwrap(),
                false
            ));
        }
        assert!(navigation_allowed_for_profile(
            &"http://127.0.0.1:1420/index.html?surface=command-approval"
                .parse()
                .unwrap(),
            true
        ));
    }
}
