use super::*;
use reqwest::{
    StatusCode,
    header::{AUTHORIZATION, HeaderValue},
};
use std::{
    io::Read,
    sync::mpsc,
    time::{Duration, Instant},
};
use url::Url;
use zeroize::Zeroizing;

const RESPONSE_LIMIT: u64 = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const CALL_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug)]
pub(super) enum AuthorityError {
    Conflict,
    Unavailable,
}

pub(super) fn endpoint(config: &SharedChatKeyConfig) -> Result<Url, ChatError> {
    let url = Url::parse(&config.anchor_endpoint).map_err(|_| ChatError::InvalidConfiguration)?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if config.anchor_endpoint.len() > 2048
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https"
            || (config.allow_loopback_http && loopback && url.scheme() == "http"))
        || url.cannot_be_a_base()
    {
        return Err(ChatError::InvalidConfiguration);
    }
    Ok(url)
}

struct Request {
    namespace: String,
    update: Option<SharedAnchorUpdate>,
    deadline: Instant,
    reply: mpsc::SyncSender<Result<SharedAnchorSnapshot, AuthorityError>>,
}

/// One bounded blocking transport worker avoids creating or dropping a blocking
/// reqwest runtime inside an async executor. Queueing and network I/O both have
/// deadlines; authority failure never falls back to process-local state.
pub(super) struct AnchorHttp {
    requests: mpsc::SyncSender<Request>,
}

impl AnchorHttp {
    pub(super) fn open(config: SharedChatKeyConfig) -> Result<Self, ChatError> {
        let url = endpoint(&config)?;
        let secret = Zeroizing::new(
            std::env::var(&config.anchor_bearer_variable)
                .map_err(|_| ChatError::StateUnavailable)?,
        );
        let mut bearer = HeaderValue::from_str(&format!("Bearer {}", secret.as_str()))
            .map_err(|_| ChatError::StateUnavailable)?;
        if secret.len() < 16 || secret.len() > 4096 || secret.chars().any(char::is_control) {
            return Err(ChatError::StateUnavailable);
        }
        bearer.set_sensitive(true);
        let (requests, receive) = mpsc::sync_channel::<Request>(16);
        let (ready, startup) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("chat-anchor-client".into())
            .spawn(move || {
                let client = client(&config);
                let _ = ready.send(client.is_ok());
                let Ok(client) = client else {
                    return;
                };
                while let Ok(request) = receive.recv() {
                    let result = if Instant::now() >= request.deadline {
                        Err(AuthorityError::Unavailable)
                    } else {
                        exchange(&client, &url, &bearer, &request)
                    };
                    let _ = request.reply.send(result);
                }
            })
            .map_err(|_| ChatError::StateUnavailable)?;
        if startup.recv_timeout(CALL_TIMEOUT) != Ok(true) {
            return Err(ChatError::StateUnavailable);
        }
        Ok(Self { requests })
    }

    pub(super) fn read(&self, namespace: &str) -> Result<SharedAnchorSnapshot, AuthorityError> {
        self.call(namespace, None)
    }

    pub(super) fn write(
        &self,
        namespace: &str,
        update: SharedAnchorUpdate,
    ) -> Result<SharedAnchorSnapshot, AuthorityError> {
        self.call(namespace, Some(update))
    }

    fn call(
        &self,
        namespace: &str,
        update: Option<SharedAnchorUpdate>,
    ) -> Result<SharedAnchorSnapshot, AuthorityError> {
        protocol::namespace(namespace).map_err(|_| AuthorityError::Unavailable)?;
        let (reply, receive) = mpsc::sync_channel(1);
        self.requests
            .try_send(Request {
                namespace: namespace.into(),
                update,
                deadline: Instant::now() + CALL_TIMEOUT,
                reply,
            })
            .map_err(|_| AuthorityError::Unavailable)?;
        receive
            .recv_timeout(CALL_TIMEOUT)
            .map_err(|_| AuthorityError::Unavailable)?
    }
}

fn client(config: &SharedChatKeyConfig) -> Result<reqwest::blocking::Client, ()> {
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(1))
        .timeout(REQUEST_TIMEOUT);
    if let Some(path) = &config.anchor_ca_pem_path {
        let file = std::fs::File::open(path).map_err(|_| ())?;
        let mut pem = Vec::new();
        file.take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut pem)
            .map_err(|_| ())?;
        if pem.len() > 4 * 1024 * 1024 {
            return Err(());
        }
        let certificates = reqwest::Certificate::from_pem_bundle(&pem).map_err(|_| ())?;
        if certificates.is_empty() {
            return Err(());
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder.build().map_err(|_| ())
}

fn exchange(
    client: &reqwest::blocking::Client,
    base: &Url,
    bearer: &HeaderValue,
    request: &Request,
) -> Result<SharedAnchorSnapshot, AuthorityError> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| AuthorityError::Unavailable)?
        .pop_if_empty()
        .push(&request.namespace);
    let builder = if let Some(update) = &request.update {
        client.post(url).json(update)
    } else {
        client.get(url)
    };
    let response = builder
        .header(AUTHORIZATION, bearer.clone())
        .send()
        .map_err(|_| AuthorityError::Unavailable)?;
    if response.status() == StatusCode::CONFLICT && request.update.is_some() {
        return Err(AuthorityError::Conflict);
    }
    if response.status() != StatusCode::OK
        || response
            .content_length()
            .is_some_and(|length| length > RESPONSE_LIMIT)
    {
        return Err(AuthorityError::Unavailable);
    }
    let mut body = Vec::new();
    response
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut body)
        .map_err(|_| AuthorityError::Unavailable)?;
    if body.len() as u64 > RESPONSE_LIMIT {
        return Err(AuthorityError::Unavailable);
    }
    let snapshot: SharedAnchorSnapshot =
        serde_json::from_slice(&body).map_err(|_| AuthorityError::Unavailable)?;
    if snapshot.version != 1 || (snapshot.revision == 0) != snapshot.anchor.is_none() {
        return Err(AuthorityError::Unavailable);
    }
    if let Some(anchor) = &snapshot.anchor {
        validate_shared_anchor_advance(None, anchor).map_err(|_| AuthorityError::Unavailable)?;
    }
    if let Some(update) = &request.update
        && (update.expected_revision.checked_add(1) != Some(snapshot.revision)
            || snapshot.anchor.as_ref() != Some(&update.anchor))
    {
        return Err(AuthorityError::Unavailable);
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests;
