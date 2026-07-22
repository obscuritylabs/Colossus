use tauri::{Runtime, UriSchemeContext, WebviewUrl, http};

pub(crate) const SCHEME: &str = "colossus-terminal";
const TERMINAL_WEBVIEW: &str = "terminal";
const MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;
const TERMINAL_CSP: &str = "default-src 'self'; script-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self'; object-src 'none'; base-uri 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; media-src 'none'; form-action 'none'";

pub(crate) fn window_url() -> WebviewUrl {
    #[cfg(debug_assertions)]
    {
        WebviewUrl::App("index.html?surface=terminal".into())
    }
    #[cfg(not(debug_assertions))]
    {
        WebviewUrl::CustomProtocol(
            format!("{SCHEME}://localhost/index.html?surface=terminal")
                .parse()
                .expect("the fixed terminal URL must be valid"),
        )
    }
}

pub(crate) fn respond<R: Runtime>(
    context: &UriSchemeContext<'_, R>,
    request: &http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    if context.webview_label() != TERMINAL_WEBVIEW {
        return error_response(http::StatusCode::FORBIDDEN);
    }
    if request.method() != http::Method::GET {
        return error_response(http::StatusCode::METHOD_NOT_ALLOWED);
    }
    let Some(path) = requested_asset(request.uri()) else {
        return error_response(http::StatusCode::NOT_FOUND);
    };
    let Some(asset) = context
        .app_handle()
        .asset_resolver()
        .get_for_scheme(path, false)
    else {
        return error_response(http::StatusCode::NOT_FOUND);
    };
    if asset.bytes().is_empty() || asset.bytes().len() > MAX_ASSET_BYTES {
        return error_response(http::StatusCode::NOT_FOUND);
    }
    let content_type = asset.mime_type().to_owned();
    secure_response(http::StatusCode::OK, &content_type, asset.bytes)
}

fn requested_asset(uri: &http::Uri) -> Option<String> {
    if uri.scheme_str() != Some(SCHEME)
        || uri.authority().map(http::uri::Authority::as_str) != Some("localhost")
    {
        return None;
    }
    match uri.path() {
        "/" | "/index.html" if uri.query() == Some("surface=terminal") => {
            Some("/index.html".into())
        }
        path if uri.query().is_none() => {
            let file_name = path.strip_prefix("/assets/")?;
            let supported_extension = std::path::Path::new(file_name)
                .extension()
                .is_some_and(|extension| extension == "js" || extension == "css");
            if file_name.is_empty()
                || file_name.len() > 192
                || file_name.contains('/')
                || file_name.contains("..")
                || !file_name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
                || !supported_extension
            {
                return None;
            }
            Some(path.into())
        }
        _ => None,
    }
}

fn secure_response(
    status: http::StatusCode,
    content_type: &str,
    body: Vec<u8>,
) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, content_type)
        .header(http::header::CACHE_CONTROL, "no-store")
        .header("Content-Security-Policy", TERMINAL_CSP)
        .header("Cross-Origin-Opener-Policy", "same-origin")
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "DENY")
        .body(body)
        .expect("the fixed terminal protocol response must be valid")
}

fn error_response(status: http::StatusCode) -> http::Response<Vec<u8>> {
    secure_response(
        status,
        "text/plain; charset=utf-8",
        b"terminal asset unavailable".to_vec(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_terminal_entrypoint_and_bounded_static_assets() {
        assert_eq!(
            requested_asset(
                &format!("{SCHEME}://localhost/index.html?surface=terminal")
                    .parse()
                    .expect("URL"),
            ),
            Some("/index.html".into())
        );
        assert_eq!(
            requested_asset(
                &format!("{SCHEME}://localhost/assets/TerminalWindow-abc_123.js")
                    .parse()
                    .expect("URL"),
            ),
            Some("/assets/TerminalWindow-abc_123.js".into())
        );
        for value in [
            "colossus-terminal://localhost/index.html?surface=main",
            "colossus-terminal://remote/index.html?surface=terminal",
            "colossus-terminal://localhost/assets/../index.html",
            "colossus-terminal://localhost/assets/chunk.js?surface=terminal",
            "https://localhost/index.html?surface=terminal",
        ] {
            assert!(
                requested_asset(&value.parse().expect("URL")).is_none(),
                "accepted {value}"
            );
        }
    }
}
