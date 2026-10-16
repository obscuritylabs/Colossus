use crate::error::{Failure, Result};
use futures::StreamExt;
use reqwest::{Method, StatusCode, header};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use url::Url;
use zeroize::Zeroizing;

#[derive(Serialize)]
struct LoginBody<'a> {
    username: &'a str,
    password: &'a str,
}

const MAX_BYTES: usize = 2 * 1024 * 1024;

pub(crate) struct Connection {
    pub origin: Url,
    client: reqwest::Client,
    cookie: Option<Zeroizing<String>>,
}

pub(crate) fn validate_origin(value: &str, development: bool) -> Result<Url> {
    let url = Url::parse(value.trim())
        .map_err(|_| Failure::new("invalid_origin", "Enter a complete HTTPS server address."))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (development && local && url.scheme() == "http"))
    {
        return Err(Failure::new(
            "invalid_origin",
            "Use the HTTPS server address without a path, username, or password.",
        ));
    }
    Ok(url)
}

impl Connection {
    pub(crate) fn new(origin: Url) -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .user_agent("Colossus-Mobile/0.1")
            .build()
            .map_err(|_| Failure::new("unavailable", "The network client could not start."))?;
        Ok(Self {
            origin,
            client,
            cookie: None,
        })
    }
    pub(crate) async fn configuration(&mut self) -> Result<Value> {
        let url = self
            .origin
            .join("api/auth/config")
            .map_err(|_| Failure::new("invalid_origin", "Invalid server address."))?;
        self.request(Method::GET, url, None, false).await
    }
    pub(crate) async fn login(
        &mut self,
        username: String,
        password: Zeroizing<String>,
    ) -> Result<Value> {
        let url = self
            .origin
            .join("auth/local")
            .map_err(|_| Failure::new("invalid_origin", "Invalid server address."))?;
        let body = Zeroizing::new(
            serde_json::to_vec(&LoginBody {
                username: &username,
                password: password.as_str(),
            })
            .map_err(|_| Failure::new("invalid_argument", "The sign-in request is invalid."))?,
        );
        self.cookie = None;
        let response = self
            .client
            .post(url)
            .header(header::ORIGIN, self.origin.origin().ascii_serialization())
            .header("X-Colossus-CSRF", "1")
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.to_vec())
            .send()
            .await
            .map_err(|_| {
                Failure::new(
                    "unavailable",
                    "Could not reach this server. Check your connection.",
                )
            })?;
        if !response.status().is_success() {
            return Err(Failure::new(
                "login_failed",
                "Sign-in failed. Check your credentials or try again later.",
            ));
        }
        let token = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .find_map(session_cookie)
            .ok_or(Failure::new(
                "login_failed",
                "This server did not issue a session.",
            ))?;
        self.cookie = Some(Zeroizing::new(token));
        let url = self
            .origin
            .join("api/me")
            .map_err(|_| Failure::new("invalid_origin", "Invalid server address."))?;
        self.request(Method::GET, url, None, true).await
    }
    pub(crate) async fn request(
        &mut self,
        method: Method,
        url: Url,
        body: Option<Value>,
        authenticated: bool,
    ) -> Result<Value> {
        if authenticated && self.cookie.is_none() {
            return Err(Failure::new("session_expired", "Sign in to continue."));
        }
        let mutation = method != Method::GET;
        let mut request = self
            .client
            .request(method, url.clone())
            .header(header::ACCEPT, "application/json");
        if authenticated && let Some(cookie) = &self.cookie {
            request = request.header(header::COOKIE, cookie.as_str());
        }
        if mutation {
            request = request
                .header(header::ORIGIN, self.origin.origin().ascii_serialization())
                .header("X-Colossus-CSRF", "1");
        }
        if let Some(body) = body {
            let bytes = serde_json::to_vec(&body)
                .map_err(|_| Failure::new("invalid_argument", "Invalid request."))?;
            if bytes.len() > 256 * 1024 {
                return Err(Failure::new(
                    "invalid_argument",
                    "This request is too large.",
                ));
            }
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(bytes);
        }
        let response = request
            .send()
            .await
            .map_err(|_| transport_failure(mutation))?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            // Project permission denial must not sign a still-authenticated user out.
            let valid = if url.path() == "/api/me" {
                false
            } else if let Some(cookie) = &self.cookie {
                let response = self
                    .client
                    .get(
                        self.origin
                            .join("api/me")
                            .map_err(|_| transport_failure(false))?,
                    )
                    .header(header::COOKIE, cookie.as_str())
                    .send()
                    .await
                    .map_err(|_| transport_failure(false))?;
                if response.status().is_success() {
                    true
                } else if matches!(
                    response.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                ) {
                    false
                } else {
                    return Err(transport_failure(false));
                }
            } else {
                false
            };
            if !valid {
                self.cookie = None;
                return Err(Failure::new(
                    "session_expired",
                    "Your session ended. Sign in again.",
                ));
            }
            return Err(Failure::new(
                "forbidden",
                "You do not have permission for this action.",
            ));
        }
        if !status.is_success() {
            return Err(match status.as_u16() {
                409 => Failure::new(
                    "conflict",
                    "The state changed. Refresh before trying again.",
                ),
                404 => Failure::new("not_found", "This resource is no longer available."),
                429 => Failure::new("busy", "The server is busy. Try again shortly."),
                400 => Failure::new("invalid_argument", "Check the request and try again."),
                _ => transport_failure(mutation),
            });
        }
        if status == StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        read_json(response)
            .await
            .map_err(|_| transport_failure(mutation))
    }
    pub(crate) async fn logout(&mut self) {
        if self.cookie.is_some()
            && let Ok(url) = self.origin.join("auth/logout")
        {
            let _ = self.request(Method::POST, url, None, true).await;
        }
        self.cookie = None;
    }
}

pub(crate) fn session_cookie(header: &str) -> Option<String> {
    let first = header.split(';').next()?;
    let token = first.strip_prefix("colossus_session=")?;
    (!token.is_empty()
        && token.len() <= 256
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=')))
    .then(|| first.to_owned())
}
fn transport_failure(mutation: bool) -> Failure {
    if mutation {
        Failure::new(
            "outcome_unknown",
            "The result could not be confirmed. Refresh and inspect the original task before submitting again.",
        )
    } else {
        Failure::new(
            "unavailable",
            "Could not reach this server. Check your connection and refresh.",
        )
    }
}
async fn read_json(response: reqwest::Response) -> Result<Value> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(part) = stream.next().await {
        let part = part.map_err(|_| transport_failure(false))?;
        if bytes.len().saturating_add(part.len()) > MAX_BYTES {
            return Err(transport_failure(false));
        }
        bytes.extend_from_slice(&part);
    }
    serde_json::from_slice(&bytes).map_err(|_| transport_failure(false))
}
