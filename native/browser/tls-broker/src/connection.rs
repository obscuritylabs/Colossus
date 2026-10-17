use crate::{authority::Authority, framing, tls::Tls};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio_rustls::{LazyConfigAcceptor, TlsConnector};

pub(crate) async fn serve(
    mut client: TcpStream,
    authority: &Authority,
    tls: &Tls,
    auth: &[u8],
    max: usize,
    deadline: std::time::Instant,
) -> Result<(), ()> {
    let connect = framing::head(&mut client).await?;
    if !bool::from(
        connect
            .one("proxy-authorization")?
            .unwrap_or_default()
            .as_bytes()
            .ct_eq(auth),
    ) {
        client.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"colossus\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.map_err(|_| ())?;
        return Ok(());
    }
    if connect.first != format!("CONNECT {} HTTP/1.1", authority.connect)
        || connect.one("host")? != Some(authority.connect.as_str())
        || connect.length()?.is_some_and(|length| length != 0)
        || connect.one("transfer-encoding")?.is_some()
        || connect.one("upgrade")?.is_some()
        || !connect.initial.is_empty()
    {
        return Err(());
    }
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .map_err(|_| ())?;
    let incoming = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), client)
        .await
        .map_err(|_| ())?;
    if !authority.admits_sni(incoming.client_hello().server_name()) {
        return Err(());
    }
    let mut client = incoming
        .into_stream(Arc::clone(&tls.downstream))
        .await
        .map_err(|_| ())?;
    if client
        .get_ref()
        .1
        .alpn_protocol()
        .is_some_and(|protocol| protocol != b"http/1.1")
    {
        return Err(());
    }
    let request = framing::head(&mut client).await?;
    let parts: Vec<_> = request.first.split(' ').collect();
    if parts.len() != 3
        || parts[2] != "HTTP/1.1"
        || !framing::token(parts[0])
        || parts[0].len() > 32
        || parts[0] == "CONNECT"
        || !parts[1].starts_with('/')
        || parts[1].starts_with("//")
        || parts[1].contains(['#', '\\'])
        || !authority.admits_host(request.one("host")?.ok_or(())?)
    {
        return Err(());
    }
    let body = framing::request_body(&mut client, &request, max).await?;
    let rewritten = request.rewritten(&request.first);
    if rewritten
        .len()
        .checked_add(body.len())
        .is_none_or(|size| size > max)
    {
        return Err(());
    }
    // Decrypted request admission precedes any upstream socket or client signing.
    let mut socket = None;
    for address in &authority.addresses {
        if let Ok(connected) = TcpStream::connect(address).await {
            socket = Some(connected);
            break;
        }
    }
    let name = rustls::pki_types::ServerName::try_from(authority.host.clone()).map_err(|_| ())?;
    let config = match &tls.identity {
        Some(identity) => identity.configure(
            &tls.upstream,
            &authority.origin,
            deadline.min(std::time::Instant::now() + std::time::Duration::from_secs(30)),
        )?,
        None => Arc::clone(&tls.upstream),
    };
    let mut upstream = TlsConnector::from(config)
        .connect(name, socket.ok_or(())?)
        .await
        .map_err(|_| ())?;
    if upstream
        .get_ref()
        .1
        .alpn_protocol()
        .is_some_and(|protocol| protocol != b"http/1.1")
    {
        return Err(());
    }
    upstream.write_all(&rewritten).await.map_err(|_| ())?;
    upstream.write_all(&body).await.map_err(|_| ())?;
    let response = framing::head(&mut upstream).await?;
    let rewritten = response.rewritten(&response.first);
    let remaining = max.checked_sub(rewritten.len()).ok_or(())?;
    let body = framing::response_body(&mut upstream, &response, parts[0], remaining).await?;
    client.write_all(&rewritten).await.map_err(|_| ())?;
    client.write_all(&body).await.map_err(|_| ())?;
    // A second client request, WebSocket byte or pipelined authority is never read
    // or forwarded. Both TLS legs end with this one validated exchange.
    client.shutdown().await.map_err(|_| ())?;
    let _ = upstream.shutdown().await;
    Ok(())
}
