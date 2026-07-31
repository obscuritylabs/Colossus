use super::*;
use std::time::Duration;

pub(super) async fn run_mcp_auth(
    runtime: &Runtime,
    action: McpAuthAction,
) -> Result<(), Box<dyn Error>> {
    match action {
        McpAuthAction::Login { server, manual } => {
            let callback_port = runtime.mcp_oauth_callback_port(&server)?;
            let listener = if manual {
                None
            } else {
                Some(TcpListener::bind(("127.0.0.1", callback_port)).await?)
            };
            let login = runtime.mcp_oauth_login_begin(&server).await?;
            eprintln!(
                "Open this authorization URL in a browser:\n{}",
                login.authorization_url
            );
            let callback_url = if let Some(listener) = listener {
                receive_oauth_callback(listener, callback_port).await?
            } else {
                eprintln!("Paste the final redirected URL and press Enter:");
                tokio::task::spawn_blocking(|| {
                    let mut value = String::new();
                    io::stdin().read_line(&mut value)?;
                    Ok::<_, io::Error>(value.trim().to_owned())
                })
                .await??
            };
            print_json(
                &runtime
                    .mcp_oauth_login_complete(&server, &callback_url)
                    .await?,
            )?;
        }
        McpAuthAction::Status { server } => {
            print_json(&runtime.mcp_oauth_status(&server).await?)?;
        }
        McpAuthAction::Logout { server } => {
            print_json(&runtime.mcp_oauth_logout(&server).await?)?;
        }
    }
    Ok(())
}

pub(super) async fn run_worker_mcp_auth(
    client: &WorkerClient,
    action: &McpAuthAction,
) -> Result<(), Box<dyn Error>> {
    match action {
        McpAuthAction::Login { server, manual } => {
            let login = client
                .call(WorkerOperation::McpAuthBegin {
                    server: server.clone(),
                })
                .await?;
            let authorization_url = login
                .get("authorization_url")
                .and_then(Value::as_str)
                .ok_or("worker returned no MCP authorization URL")?;
            let callback_url = login
                .get("callback_url")
                .and_then(Value::as_str)
                .ok_or("worker returned no MCP callback URL")?;
            let callback = url::Url::parse(callback_url)?;
            let port = callback
                .port()
                .ok_or("worker returned an invalid MCP callback port")?;
            let listener = if *manual {
                None
            } else {
                Some(TcpListener::bind(("127.0.0.1", port)).await?)
            };
            eprintln!("Open this authorization URL in a browser:\n{authorization_url}");
            let callback_url = if let Some(listener) = listener {
                receive_oauth_callback(listener, port).await?
            } else {
                eprintln!("Paste the final redirected URL and press Enter:");
                tokio::task::spawn_blocking(|| {
                    let mut value = String::new();
                    io::stdin().read_line(&mut value)?;
                    Ok::<_, io::Error>(value.trim().to_owned())
                })
                .await??
            };
            print_json(
                &client
                    .call(WorkerOperation::McpAuthComplete {
                        server: server.clone(),
                        callback_url,
                    })
                    .await?,
            )?;
        }
        McpAuthAction::Status { server } => {
            print_json(
                &client
                    .call(WorkerOperation::McpAuthStatus {
                        server: server.clone(),
                    })
                    .await?,
            )?;
        }
        McpAuthAction::Logout { server } => {
            print_json(
                &client
                    .call(WorkerOperation::McpAuthLogout {
                        server: server.clone(),
                    })
                    .await?,
            )?;
        }
    }
    Ok(())
}

async fn receive_oauth_callback(
    listener: TcpListener,
    port: u16,
) -> Result<String, Box<dyn Error>> {
    let (mut stream, address) = tokio::time::timeout(Duration::from_secs(300), listener.accept())
        .await
        .map_err(|_| "OAuth callback timed out")??;
    if !address.ip().is_loopback() {
        return Err("OAuth callback did not originate from loopback".into());
    }
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 1024];
        let count = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .map_err(|_| "OAuth callback request timed out")??;
        if count == 0 {
            return Err("OAuth callback closed before its request completed".into());
        }
        if request.len().saturating_add(count) > 16 * 1024 {
            return Err("OAuth callback request exceeded 16 KiB".into());
        }
        request.extend_from_slice(&chunk[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&request)?;
    let mut fields = request
        .lines()
        .next()
        .ok_or("OAuth callback request line is absent")?
        .split_ascii_whitespace();
    if fields.next() != Some("GET") {
        return Err("OAuth callback requires GET".into());
    }
    let target = fields.next().ok_or("OAuth callback target is absent")?;
    if !target.starts_with("/callback?") {
        return Err("OAuth callback target is invalid".into());
    }
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 55\r\nConnection: close\r\n\r\nAuthorization complete. You may close this browser tab.",
        )
        .await?;
    Ok(format!("http://127.0.0.1:{port}{target}"))
}
