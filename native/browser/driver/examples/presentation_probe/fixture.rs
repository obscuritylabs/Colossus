//! Synthetic bounded fixed-origin proxy; never forwards traffic to the network.
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    task::{AbortHandle, JoinSet},
};

pub const ORIGIN: &str = "http://colossus-presentation-fixture.invalid";
pub const PASSWORD: &str = "presentation-fixture";
const AUTHORIZATION: &str = "Basic Y29sb3NzdXM6cHJlc2VudGF0aW9uLWZpeHR1cmU=";
const PAGE: &str = r#"<!doctype html><html><head><title>Native presentation fixture</title></head><body style="background:#c8e6ee"><h1>Native presentation fixture</h1><input aria-label="Ordinary native input" style="position:absolute;left:40px;top:80px;width:300px;height:40px" autocomplete="off"><button onclick="setTimeout(()=>location.href='/fixture.html?recovery=1',250)" style="position:absolute;left:40px;top:150px">Native button</button></body></html>"#;

pub struct Fixture {
    pub port: u16,
    pub requests: Arc<AtomicU64>,
    pub recovery_requests: Arc<AtomicU64>,
    pub recovery_released: Arc<AtomicBool>,
    worker: AbortHandle,
}
impl Fixture {
    pub async fn start() -> Result<Self, &'static str> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "fixture proxy unavailable")?;
        let port = listener
            .local_addr()
            .map_err(|_| "fixture proxy address unavailable")?
            .port();
        let requests = Arc::new(AtomicU64::new(0));
        let seen = Arc::clone(&requests);
        let recovery_requests = Arc::new(AtomicU64::new(0));
        let recovery = Arc::clone(&recovery_requests);
        let recovery_released = Arc::new(AtomicBool::new(false));
        let release = Arc::clone(&recovery_released);
        let worker = tokio::spawn(async move {
            let mut sockets = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break; };
                        if sockets.len() >= 16 { continue; }
                        let seen = Arc::clone(&seen);
                        let recovery = Arc::clone(&recovery);
                        let release = Arc::clone(&release);
                        sockets.spawn(async move {
                            let _ = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                                let mut header = Vec::new(); let mut byte = [0_u8; 1];
                                while !header.ends_with(b"\r\n\r\n") {
                                    if header.len() >= 16 * 1024 || stream.read_exact(&mut byte).await.is_err() { return; }
                                    header.push(byte[0]);
                                }
                                let Ok(header) = std::str::from_utf8(&header) else { return; };
                                let authorized = header.lines().any(|line| line.split_once(':').is_some_and(|(name,value)| name.eq_ignore_ascii_case("proxy-authorization") && value.trim() == AUTHORIZATION));
                                let response = if !authorized {
                                    "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"presentation-fixture\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                                } else if [format!("GET {ORIGIN}/fixture.html HTTP/1.1"), format!("GET {ORIGIN}/fixture.html?handoff=1 HTTP/1.1"), format!("GET {ORIGIN}/fixture.html?recovery=1 HTTP/1.1")].iter().any(|expected| header.lines().next() == Some(expected.as_str())) {
                                    seen.fetch_add(1, Ordering::AcqRel);
                                    if header.lines().next() == Some(format!("GET {ORIGIN}/fixture.html?recovery=1 HTTP/1.1").as_str()) {
                                        recovery.fetch_add(1, Ordering::AcqRel);
                                        while !release.load(Ordering::Acquire) { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
                                    }
                                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}", PAGE.len())
                                } else { "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned() };
                                let _ = stream.write_all(response.as_bytes()).await;
                            }).await;
                        });
                    }
                    _ = sockets.join_next(), if !sockets.is_empty() => {}
                }
            }
        });
        Ok(Self {
            port,
            requests,
            recovery_requests,
            recovery_released,
            worker: worker.abort_handle(),
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
