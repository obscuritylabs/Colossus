//! Fixed review document and bounded bundled assets, isolated from guest pages.

use tauri::{Runtime, UriSchemeContext, WebviewUrl, http};

pub(crate) const SCHEME: &str = "colossus-client-identity";
const QUERY: &str = "surface=client-identity-review";
const WINDOWS_HOST: &str = "colossus-client-identity.localhost";
const CSP: &str = "default-src 'self'; script-src 'self'; connect-src ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; font-src 'self'; object-src 'none'; base-uri 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; media-src 'none'; form-action 'none'";

pub(crate) fn window_url() -> WebviewUrl {
    if cfg!(all(
        debug_assertions,
        not(any(
            feature = "embedded-chromium-preview",
            feature = "macos-contained-browser-diagnostic",
            feature = "windows-browser-development",
            feature = "windows-browser-functional-preview"
        ))
    )) {
        WebviewUrl::App(format!("index.html?{QUERY}").into())
    } else {
        WebviewUrl::CustomProtocol(
            format!("{SCHEME}://localhost/index.html?{QUERY}")
                .parse()
                .expect("fixed identity review URL"),
        )
    }
}

pub(crate) fn navigation_allowed(url: &tauri::Url) -> bool {
    navigation_allowed_for_profile(url, matches!(window_url(), WebviewUrl::App(_)))
}

pub(super) fn document_authorized(label: &str, parent: &str, url: &tauri::Url) -> bool {
    label == super::WEBVIEW && parent == "main" && navigation_allowed(url)
}

fn navigation_allowed_for_profile(url: &tauri::Url, debug: bool) -> bool {
    let bundled = url.port().is_none()
        && ((url.scheme() == SCHEME && url.host_str() == Some("localhost"))
            || (url.scheme() == "http" && url.host_str() == Some(WINDOWS_HOST)));
    let development = debug
        && ((url.scheme() == "tauri"
            && url.host_str() == Some("localhost")
            && url.port().is_none())
            || (url.scheme() == "http"
                && url.host_str() == Some("tauri.localhost")
                && url.port().is_none())
            || (url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port() == Some(1420)));
    (bundled || development)
        && url.path() == "/index.html"
        && url.query() == Some(QUERY)
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

fn requested_asset(uri: &http::Uri) -> Option<&str> {
    if !matches!(
        (
            uri.scheme_str(),
            uri.authority().map(http::uri::Authority::as_str)
        ),
        (Some(SCHEME), Some("localhost")) | (Some("http"), Some(WINDOWS_HOST))
    ) {
        return None;
    }
    let path = uri.path();
    if path == "/index.html" && uri.query() == Some(QUERY) {
        return Some(path);
    }
    let name = path.strip_prefix("/assets/")?;
    (uri.query().is_none()
        && !name.is_empty()
        && name.len() <= 192
        && !name.contains("..")
        && !name.contains('/')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        && std::path::Path::new(name)
            .extension()
            .is_some_and(|extension| extension == "js" || extension == "css"))
    .then_some(path)
}

pub(crate) fn respond<R: Runtime>(
    context: &UriSchemeContext<'_, R>,
    request: &http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let resolved = (context.webview_label() == super::WEBVIEW
        && request.method() == http::Method::GET)
        .then(|| requested_asset(request.uri()))
        .flatten()
        .and_then(|path| {
            context
                .app_handle()
                .asset_resolver()
                .get_for_scheme(path.to_owned(), false)
        });
    let (status, mime, body) = match resolved {
        Some(asset) if !asset.bytes().is_empty() && asset.bytes().len() <= 8 * 1024 * 1024 => (
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
        .header("Content-Security-Policy", CSP)
        .header("Cross-Origin-Opener-Policy", "same-origin")
        .header("X-Frame-Options", "DENY")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .body(body)
        .expect("fixed identity review response headers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_local_document_and_bounded_assets_are_the_entire_surface() {
        for origin in [
            "colossus-client-identity://localhost",
            "http://colossus-client-identity.localhost",
        ] {
            let document = format!("{origin}/index.html?{QUERY}");
            assert!(navigation_allowed_for_profile(
                &document.parse().unwrap(),
                false
            ));
            assert_eq!(
                requested_asset(&document.parse().unwrap()),
                Some("/index.html")
            );
            assert_eq!(
                requested_asset(&format!("{origin}/assets/review-A_b.js").parse().unwrap()),
                Some("/assets/review-A_b.js")
            );
            assert!(!navigation_allowed_for_profile(
                &format!("{document}#fake").parse().unwrap(),
                false
            ));
        }
        for url in [
            "https://colossus-client-identity.localhost/index.html?surface=client-identity-review",
            "colossus-client-identity://user@localhost/index.html?surface=client-identity-review",
            "colossus-client-identity://localhost:80/index.html?surface=client-identity-review",
            "colossus-client-identity://localhost/?surface=client-identity-review",
            "colossus-client-identity://localhost/index.html?surface=client-identity-review&token=fake",
            "colossus-client-identity://localhost/index.html?surface=terminal",
            "https://example.com/index.html?surface=client-identity-review",
        ] {
            assert!(
                !navigation_allowed_for_profile(&url.parse().unwrap(), false),
                "{url}"
            );
        }
        for path in [
            "/assets/../index.html",
            "/assets/review.js?scope=forged",
            "/assets/review/map.js",
            "/assets/review.svg",
            "/assets/review%2ejs",
            "/index.html",
        ] {
            assert!(
                requested_asset(&format!("{SCHEME}://localhost{path}").parse().unwrap()).is_none(),
                "{path}"
            );
        }
        let dev = "http://127.0.0.1:1420/index.html?surface=client-identity-review"
            .parse()
            .unwrap();
        assert!(!navigation_allowed_for_profile(&dev, false));
        assert!(navigation_allowed_for_profile(&dev, true));
        assert!(CSP.contains("connect-src ipc: http://ipc.localhost;"));
        assert!(CSP.contains("frame-src 'none'; child-src 'none'"));
    }

    #[test]
    fn document_identity_cannot_be_replaced_by_a_main_or_guest_label() {
        let url = "colossus-client-identity://localhost/index.html?surface=client-identity-review"
            .parse()
            .unwrap();
        assert!(document_authorized(super::super::WEBVIEW, "main", &url));
        for (label, parent) in [
            ("main", "main"),
            ("browser-guest-1", "main"),
            ("client-identity-review", "guest"),
            ("command-approval", "main"),
        ] {
            assert!(!document_authorized(label, parent, &url));
        }
        assert!(!document_authorized(
            super::super::WEBVIEW,
            "main",
            &"https://example.com/index.html?surface=client-identity-review"
                .parse()
                .unwrap()
        ));
    }
}
