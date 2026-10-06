//! Public frontend shell routes; resource authorization stays in the API handlers.

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{Method, StatusCode},
    response::IntoResponse,
};
use std::path::Path;
use tower_http::services::{ServeDir, ServeFile};

pub(super) fn service(web_root: &Path) -> Router {
    let assets = ServeDir::new(web_root);
    let shell = ServeFile::new(web_root.join("index.html"));
    Router::new().fallback(move |request: Request| {
        let mut assets = assets.clone();
        let mut shell = shell.clone();
        async move {
            // Preserve the static service's existing method rejection and Allow header.
            if request.method() != Method::GET && request.method() != Method::HEAD {
                return match assets.try_call(request).await {
                    Ok(response) => response.map(Body::new),
                    Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                };
            }
            let Some(parts) = path_parts(request.uri().path()) else {
                return StatusCode::NOT_FOUND.into_response();
            };
            if frontend_path(&parts) {
                return match shell.try_call(request).await {
                    Ok(response) => response.map(Body::new),
                    Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                };
            }
            if parts.first().is_some_and(|part| {
                matches!(
                    part.as_str(),
                    "api" | "auth" | "health" | "projects" | "fleet" | "admin" | "settings"
                )
            }) {
                return StatusCode::NOT_FOUND.into_response();
            }
            match assets.try_call(request).await {
                Ok(response) => response.map(Body::new),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
    })
}

fn path_parts(path: &str) -> Option<Vec<String>> {
    if path.len() > 2048 || !path.starts_with('/') {
        return None;
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    if path.is_empty() {
        return Some(Vec::new());
    }
    path.strip_prefix('/')?
        .split('/')
        .map(|part| {
            let decoded = decode_part(part)?;
            (!decoded.is_empty()
                && decoded != "."
                && decoded != ".."
                && !decoded
                    .chars()
                    .any(|c| c == '/' || c == '\\' || c.is_control()))
            .then_some(decoded)
        })
        .collect()
}

fn decode_part(part: &str) -> Option<String> {
    let bytes = part.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = (*bytes.get(index + 1)? as char).to_digit(16)?;
            let low = (*bytes.get(index + 2)? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.encode_utf16().count() <= 256
        && !value.chars().any(|c| c.is_whitespace() || c == '\u{feff}')
}

fn frontend_path(parts: &[String]) -> bool {
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    match parts.as_slice() {
        [] | ["fleet" | "projects" | "admin" | "settings"] => true,
        ["admin", "users" | "projects" | "settings"] => true,
        [
            "projects",
            project,
            "overview" | "tasks" | "analytics" | "access" | "settings",
        ] => identifier(project),
        [
            "projects",
            project,
            "agents",
            node,
            "overview" | "threads" | "analytics" | "policy",
        ] => identifier(project) && identifier(node),
        ["projects", project, "threads" | "tasks", resource] => {
            identifier(project) && identifier(resource)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
