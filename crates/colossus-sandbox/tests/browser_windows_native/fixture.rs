//! Real bounded loopback HTTP fixtures; no personal profile or network service.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};

pub struct Fixture {
    pub origin: String,
    pub denied: String,
    pub allowed_requests: Arc<AtomicUsize>,
    pub denied_requests: Arc<AtomicUsize>,
    pub uploaded: Arc<AtomicUsize>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Fixture {
    pub async fn start() -> Self {
        let allowed = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("allowed fixture bind");
        let denied = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("denied fixture bind");
        let origin = format!("http://localhost:{}", allowed.local_addr().unwrap().port());
        let denied_url = format!("http://127.0.0.1:{}", denied.local_addr().unwrap().port());
        let page = format!(
            r#"<!doctype html><html><head><title>Owned Windows fixture</title></head>
<body style="background:#c8e6ee"><h1>Owned Windows fixture</h1>
<input aria-label="Ordinary native input" style="position:absolute;left:40px;top:80px;width:300px;height:40px" autocomplete="off">
<input aria-label="Private test field" type="password" value="synthetic-protected-value">
<img src="{denied_url}/blocked"><img src="http://unreviewed-dns.invalid/blocked">
<form id="owned-upload" method="post" action="/upload" enctype="multipart/form-data">
<input type="file" name="owned_file" aria-label="Owned native file input">
<button type="submit">Submit owned upload</button></form>
<a href="/download-redirect">Owned binary download</a>
<a href="/download-empty">Owned empty download</a>
<a href="{denied_url}/download">Forbidden binary download</a>
<output id="uploaded">Owned upload waiting</output>
<script>document.querySelector('#owned-upload').addEventListener('submit', async event => {{
event.preventDefault(); const response = await fetch('/upload', {{method:'POST', body:new FormData(event.target)}});
document.querySelector('#uploaded').textContent = response.ok ? 'Owned upload received' : 'Owned upload rejected';
}});</script>
</body></html>"#
        );
        let allowed_requests = Arc::new(AtomicUsize::new(0));
        let denied_requests = Arc::new(AtomicUsize::new(0));
        let uploaded = Arc::new(AtomicUsize::new(0));
        let tasks = vec![
            serve(
                allowed,
                page,
                Arc::clone(&allowed_requests),
                Some((origin.clone(), Arc::clone(&uploaded))),
            ),
            serve(
                denied,
                "denied fixture".into(),
                Arc::clone(&denied_requests),
                None,
            ),
        ];
        Self {
            origin,
            denied: denied_url,
            allowed_requests,
            denied_requests,
            uploaded,
            tasks,
        }
    }
    pub async fn finish(mut self) {
        for task in &mut self.tasks {
            task.abort();
            let _ = task.await;
        }
        self.tasks.clear();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
fn serve(
    listener: TcpListener,
    page: String,
    observed: Arc<AtomicUsize>,
    transfers: Option<(String, Arc<AtomicUsize>)>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((mut connection, _)) = listener.accept().await else {
                break;
            };
            let mut header = zeroize::Zeroizing::new(Vec::new());
            let mut buffer = [0; 1024];
            while !header.windows(4).any(|part| part == b"\r\n\r\n") && header.len() <= 8192 {
                let Ok(Ok(count)) = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    connection.read(&mut buffer),
                )
                .await
                else {
                    break;
                };
                if count == 0 {
                    break;
                }
                header.extend_from_slice(&buffer[..count]);
            }
            if !header.windows(4).any(|part| part == b"\r\n\r\n") || header.len() > 8192 {
                continue;
            }
            observed.fetch_add(1, Ordering::SeqCst);
            let header_end = header
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap()
                + 4;
            let Ok(text) = std::str::from_utf8(&header[..header_end]) else {
                continue;
            };
            let request = text.lines().next().unwrap_or("");
            let mut parts = request.split_whitespace();
            let method = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            let path = transfers.as_ref().map_or(target, |(origin, _)| {
                target.strip_prefix(origin).unwrap_or(target)
            });
            if method == "GET" && path == "/download-redirect" && transfers.is_some() {
                let _ = connection.write_all(b"HTTP/1.1 302 Found\r\nLocation: /download-final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                let _ = connection.shutdown().await;
                continue;
            }
            if method == "GET"
                && matches!(path, "/download-final" | "/download-empty")
                && transfers.is_some()
            {
                let bytes = if path == "/download-final" {
                    super::transfer::download_bytes()
                } else {
                    zeroize::Zeroizing::new(Vec::new())
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"../../untrusted.bin\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = connection.write_all(response.as_bytes()).await;
                let _ = connection.write_all(&bytes).await;
                let _ = connection.shutdown().await;
                continue;
            }
            if method == "POST" && path == "/upload" {
                let content_length = text.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("Content-Length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                });
                let boundary = text.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if !name.eq_ignore_ascii_case("Content-Type") {
                        return None;
                    }
                    value
                        .trim()
                        .strip_prefix("multipart/form-data; boundary=")
                        .map(str::to_owned)
                });
                let Some(length) =
                    content_length.filter(|length| *length <= 4 * 1024 * 1024 + 64 * 1024)
                else {
                    continue;
                };
                while header.len() < header_end + length {
                    let remaining = (header_end + length - header.len()).min(buffer.len());
                    let Ok(Ok(count)) = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        connection.read(&mut buffer[..remaining]),
                    )
                    .await
                    else {
                        break;
                    };
                    if count == 0 {
                        break;
                    }
                    header.extend_from_slice(&buffer[..count]);
                }
                let accepted = header.len() == header_end + length
                    && boundary
                        .as_deref()
                        .is_some_and(|boundary| multipart(&header[header_end..], boundary))
                    && transfers.is_some();
                if accepted {
                    transfers.as_ref().unwrap().1.fetch_add(1, Ordering::SeqCst);
                }
                let response = if accepted {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice()
                } else {
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .as_slice()
                };
                let _ = connection.write_all(response).await;
                let _ = connection.shutdown().await;
                continue;
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                page.len(),
                page
            );
            let _ = connection.write_all(response.as_bytes()).await;
            let _ = connection.shutdown().await;
        }
    })
}

fn multipart(body: &[u8], boundary: &str) -> bool {
    if boundary.is_empty()
        || boundary.len() > 128
        || !boundary
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return false;
    }
    let prefix = format!("--{boundary}\r\n");
    let suffix = format!("\r\n--{boundary}--\r\n");
    if !body.starts_with(prefix.as_bytes()) || !body.ends_with(suffix.as_bytes()) {
        return false;
    }
    let Some(header_end) = body.windows(4).position(|part| part == b"\r\n\r\n") else {
        return false;
    };
    let Ok(headers) = std::str::from_utf8(&body[prefix.len()..header_end]) else {
        return false;
    };
    if !headers
        .contains("Content-Disposition: form-data; name=\"owned_file\"; filename=\"upload.txt\"")
    {
        return false;
    }
    body.get(header_end + 4..body.len() - suffix.len())
        .is_some_and(|payload| payload == *super::transfer::upload_bytes())
}
