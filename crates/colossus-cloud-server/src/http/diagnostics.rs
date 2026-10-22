//! Bounded operator diagnostics without caller-controlled request content.

use axum::{
    extract::{MatchedPath, Request},
    http::{Method, header},
    response::Response,
};

pub(super) enum FailureCategory {
    Handler,
    AdmissionTimeout,
}

pub(super) struct RequestFailure {
    method: &'static str,
    route: Option<MatchedPath>,
}

impl RequestFailure {
    pub(super) fn capture(request: &Request) -> Self {
        Self {
            // HTTP permits extension methods; never copy their arbitrary bytes.
            method: match *request.method() {
                Method::GET => "GET",
                Method::HEAD => "HEAD",
                Method::POST => "POST",
                Method::PUT => "PUT",
                Method::DELETE => "DELETE",
                Method::CONNECT => "CONNECT",
                Method::OPTIONS => "OPTIONS",
                Method::TRACE => "TRACE",
                Method::PATCH => "PATCH",
                _ => "OTHER",
            },
            route: request.extensions().get::<MatchedPath>().cloned(),
        }
    }

    pub(super) fn report(self, response: &Response, category: FailureCategory) {
        if let Some(line) = self.line(response, category) {
            eprintln!("{line}");
        }
    }

    fn line(&self, response: &Response, category: FailureCategory) -> Option<String> {
        let status = response.status();
        let json = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .and_then(|media| media.trim().split_once('/'))
            .is_some_and(|(kind, subtype)| {
                kind.eq_ignore_ascii_case("application")
                    && (subtype.eq_ignore_ascii_case("json")
                        || subtype
                            .rsplit_once('+')
                            .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("json")))
            });
        // Axum extractor/route rejections may not carry the ordinary JSON error
        // envelope; the Web client otherwise displays the same generic failure.
        if !status.is_server_error() && (!status.is_client_error() || json) {
            return None;
        }
        // MatchedPath is the router's template, never the URI with identifiers or
        // query parameters. Keep even program-owned template output on one line.
        let route = self
            .route
            .as_ref()
            .map(MatchedPath::as_str)
            .filter(|route| route.len() <= 256 && route.bytes().all(|byte| byte.is_ascii_graphic()))
            .unwrap_or("<unmatched>");
        let category = match category {
            FailureCategory::Handler => "handler",
            FailureCategory::AdmissionTimeout => "admission_timeout",
        };
        Some(format!(
            "Cloud HTTP request failed: method={} route={} status={} category={category}",
            self.method,
            route,
            status.as_u16()
        ))
    }
}

#[cfg(test)]
mod tests;
