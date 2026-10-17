//! One bounded proxy exchange. No borrowed page/URL bytes become diagnostics.

use crate::{
    BASE64, HmacSha256, MAX_PROXY_HEADER_BYTES,
    http_proxy::{connect_destination, single_header_value},
    proxy_tls::{read_tls_client_hello, tls_server_name},
};
use base64::Engine as _;
use colossus_contracts::HostSecret;
use hmac::Mac as _;
use reqwest::Url;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use zeroize::Zeroizing;

const MAX_PLAINTEXT_BODY_BYTES: u64 = 1024 * 1024;

pub(super) async fn serve(
    mut client: TcpStream,
    origins: &[String],
    credential: &HostSecret,
    max_bytes: u64,
) -> Result<(), ()> {
    let (captured, header_end) = read_header(&mut client).await?;
    let header = std::str::from_utf8(&captured[..header_end]).map_err(|_| ())?;
    let fields = parse_header(header)?;
    let authorization = single_header_value(header, "proxy-authorization").map_err(|_| ())?;
    if !authenticated(authorization, credential)? {
        client.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"colossus\"\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await.map_err(|_| ())?;
        return Ok(());
    }
    let first = fields.first().ok_or(())?;
    let request: Vec<_> = first.split(' ').collect();
    if request.len() != 3
        || !valid_token(request[0])
        || request[0].len() > 32
        || !matches!(request[2], "HTTP/1.0" | "HTTP/1.1")
    {
        return Err(());
    }
    if request[0] == "CONNECT" {
        tunnel(
            client,
            origins,
            request[1],
            &captured[header_end..],
            max_bytes,
        )
        .await
    } else {
        plaintext(
            client,
            origins,
            &request,
            header,
            &fields,
            &captured[header_end..],
            max_bytes,
        )
        .await
    }
}

async fn read_header(client: &mut TcpStream) -> Result<(Zeroizing<Vec<u8>>, usize), ()> {
    let mut captured = Zeroizing::new(Vec::new());
    loop {
        if let Some(offset) = captured.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            return Ok((captured, offset + 4));
        }
        let mut buffer = [0_u8; 1024];
        let count = client.read(&mut buffer).await.map_err(|_| ())?;
        if count == 0 || captured.len() + count > MAX_PROXY_HEADER_BYTES {
            return Err(());
        }
        captured.extend_from_slice(&buffer[..count]);
    }
}

fn parse_header(header: &str) -> Result<Vec<&str>, ()> {
    let fields: Vec<_> = header
        .strip_suffix("\r\n\r\n")
        .ok_or(())?
        .split("\r\n")
        .collect();
    if fields.iter().any(|line| {
        line.bytes()
            .any(|byte| !matches!(byte, 0x20..=0x7e | b'\t'))
    }) {
        return Err(());
    }
    for line in fields.iter().skip(1) {
        let (name, value) = line.split_once(':').ok_or(())?;
        if !valid_token(name) || value.trim().is_empty() {
            return Err(());
        }
    }
    Ok(fields)
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn authenticated(value: Option<&str>, credential: &HostSecret) -> Result<bool, ()> {
    let plain = Zeroizing::new(format!("colossus:{}", credential.expose()));
    let expected = Zeroizing::new(format!("Basic {}", BASE64.encode(plain.as_bytes())));
    let mut expected_mac =
        HmacSha256::new_from_slice(b"colossus-browser-proxy-comparison-v1").map_err(|_| ())?;
    expected_mac.update(expected.as_bytes());
    let mut provided_mac =
        HmacSha256::new_from_slice(b"colossus-browser-proxy-comparison-v1").map_err(|_| ())?;
    provided_mac.update(value.unwrap_or_default().as_bytes());
    Ok(provided_mac
        .verify_slice(&expected_mac.finalize().into_bytes())
        .is_ok())
}

async fn tunnel(
    mut client: TcpStream,
    origins: &[String],
    authority: &str,
    initial: &[u8],
    max_bytes: u64,
) -> Result<(), ()> {
    // Authority syntax is closed; userinfo/path/query and canonicalization tricks
    // cannot be accepted by the generic URL parser as a different CONNECT target.
    let url = Url::parse(&format!("https://{authority}/")).map_err(|_| ())?;
    if authority.contains(['/', '?', '#', '@', '\\'])
        || authority.is_empty()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(());
    }
    if !origins.contains(&url.origin().ascii_serialization()) {
        return forbidden(&mut client).await;
    }
    let host = url.host_str().ok_or(())?;
    let host = unbracketed(host);
    let port = url.port_or_known_default().ok_or(())?;
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .map_err(|_| ())?;
    let hello = Zeroizing::new(
        read_tls_client_hello(&mut client, initial)
            .await
            .map_err(|_| ())?,
    );
    if hello.len() as u64 >= max_bytes {
        return Err(());
    }
    let sni = tls_server_name(&hello).map_err(|_| ())?;
    let valid_server_name = match host.parse::<std::net::IpAddr>() {
        // Browsers omit SNI for literal IP destinations. A different DNS name
        // must not select an unrelated virtual host behind the permitted IP.
        Ok(_) => sni.is_none(),
        Err(_) => sni.is_some_and(|name| name.eq_ignore_ascii_case(host)),
    };
    if !valid_server_name {
        return Err(());
    }
    let mut upstream = connect(host, port).await?;
    upstream.write_all(&hello).await.map_err(|_| ())?;
    let (read_client, mut write_client) = client.split();
    let (read_upstream, mut write_upstream) = upstream.split();
    let remaining_client_bytes = max_bytes - hello.len() as u64;
    let mut read_client = read_client.take(remaining_client_bytes);
    let mut read_upstream = read_upstream.take(max_bytes);
    let client_to_upstream = async {
        let count = tokio::io::copy(&mut read_client, &mut write_upstream)
            .await
            .map_err(|_| ())?;
        write_upstream.shutdown().await.map_err(|_| ())?;
        if count == remaining_client_bytes {
            Err(())
        } else {
            Ok(())
        }
    };
    let upstream_to_client = async {
        let count = tokio::io::copy(&mut read_upstream, &mut write_client)
            .await
            .map_err(|_| ())?;
        write_client.shutdown().await.map_err(|_| ())?;
        if count == max_bytes { Err(()) } else { Ok(()) }
    };
    tokio::try_join!(client_to_upstream, upstream_to_client)?;
    Ok(())
}

async fn plaintext(
    mut client: TcpStream,
    origins: &[String],
    request: &[&str],
    header: &str,
    fields: &[&str],
    initial_body: &[u8],
    max_bytes: u64,
) -> Result<(), ()> {
    let url = Url::parse(request[1]).map_err(|_| ())?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(());
    }
    if !origins.contains(&url.origin().ascii_serialization()) {
        return forbidden(&mut client).await;
    }
    let host_header = single_header_value(header, "host")
        .map_err(|_| ())?
        .ok_or(())?;
    let host_url = Url::parse(&format!("http://{host_header}/")).map_err(|_| ())?;
    if host_header.contains(['/', '?', '#', '@', '\\']) || host_url.origin() != url.origin() {
        return Err(());
    }
    for denied in ["transfer-encoding", "upgrade", "expect"] {
        if single_header_value(header, denied)
            .map_err(|_| ())?
            .is_some()
        {
            return Err(());
        }
    }
    let body_len = match single_header_value(header, "content-length").map_err(|_| ())? {
        Some(value) if value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse::<u64>().map_err(|_| ())?
        }
        Some(_) => return Err(()),
        None => 0,
    };
    if body_len > MAX_PLAINTEXT_BODY_BYTES || initial_body.len() as u64 > body_len {
        return Err(());
    }
    let host = unbracketed(url.host_str().ok_or(())?);
    let port = url.port_or_known_default().ok_or(())?;
    let target = url.query().map_or_else(
        || url.path().to_owned(),
        |query| format!("{}?{query}", url.path()),
    );
    let mut rewritten = Zeroizing::new(format!("{} {target} HTTP/1.1\r\n", request[0]));
    for line in fields.iter().skip(1) {
        let name = line.split_once(':').ok_or(())?.0;
        if ["proxy-authorization", "proxy-connection", "connection"]
            .iter()
            .any(|excluded| name.eq_ignore_ascii_case(excluded))
        {
            continue;
        }
        rewritten.push_str(line);
        rewritten.push_str("\r\n");
    }
    rewritten.push_str("Connection: close\r\n\r\n");
    if rewritten.len() as u64 + body_len > max_bytes {
        return Err(());
    }
    let mut upstream = connect(host, port).await?;
    upstream
        .write_all(rewritten.as_bytes())
        .await
        .map_err(|_| ())?;
    upstream.write_all(initial_body).await.map_err(|_| ())?;
    let remaining = body_len - initial_body.len() as u64;
    let copied = tokio::io::copy(&mut (&mut client).take(remaining), &mut upstream)
        .await
        .map_err(|_| ())?;
    if copied != remaining {
        return Err(());
    }
    // No subsequent client bytes are forwarded. This closes the HTTP keepalive /
    // pipelining bypass in raw bidirectional process proxy transports.
    let count = tokio::io::copy(&mut upstream.take(max_bytes), &mut client)
        .await
        .map_err(|_| ())?;
    client.shutdown().await.map_err(|_| ())?;
    if count == max_bytes { Err(()) } else { Ok(()) }
}

fn unbracketed(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

fn permits_private(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || colossus_network::parse_host_ip(host)
            .is_some_and(colossus_network::non_public_network_address)
}

async fn connect(host: &str, port: u16) -> Result<TcpStream, ()> {
    let mut pinned = crate::http_proxy::resolve_destinations(host, port, permits_private(host))
        .await
        .map_err(|_| ())?;
    // `localhost` is an exact loopback authority, not a grant for arbitrary
    // private destinations that a changed resolver could return under that name.
    if host.eq_ignore_ascii_case("localhost") {
        pinned.retain(|address| address.ip().is_loopback());
    }
    connect_destination(host, port, Some(&pinned), permits_private(host))
        .await
        .map_err(|_| ())
}

async fn forbidden(client: &mut TcpStream) -> Result<(), ()> {
    client
        .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
        .await
        .map_err(|_| ())
}
