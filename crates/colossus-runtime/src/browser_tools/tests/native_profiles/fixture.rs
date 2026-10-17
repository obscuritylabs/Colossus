use super::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

type Receipts = Arc<StdMutex<Vec<(String, bool)>>>;

pub(super) async fn start() -> (String, Receipts, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let receipts: Receipts = Arc::new(StdMutex::new(Vec::new()));
    let observed = receipts.clone();
    let marker = Uuid::now_v7().simple().to_string();
    let serving = tokio::spawn(async move {
        loop {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            while !bytes.windows(4).any(|value| value == b"\r\n\r\n") {
                let count =
                    tokio::time::timeout(Duration::from_secs(2), connection.read(&mut buffer))
                        .await
                        .unwrap()
                        .unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() <= 8192);
            }
            let header = std::str::from_utf8(&bytes).unwrap();
            let path = header.split_whitespace().nth(1).unwrap_or("");
            let cookie = header.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.eq_ignore_ascii_case("cookie")
                        && value
                            .split(';')
                            .any(|part| part.trim() == format!("colossus_fixture={marker}"))
                })
            });
            assert!(matches!(path, "/seed" | "/observe" | "/favicon.ico"));
            if path == "/favicon.ico" {
                connection
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
                continue;
            }
            observed.lock().unwrap().push((path.into(), cookie));
            let title = if cookie {
                "Cookie present"
            } else {
                "Cookie absent"
            };
            let page = format!("<!doctype html><title>{title}</title><label>{title}</label>");
            let setting = if path == "/seed" {
                format!(
                    "Set-Cookie: colossus_fixture={marker}; Max-Age=86400; Path=/; HttpOnly; SameSite=Strict\r\n"
                )
            } else {
                String::new()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n{setting}Connection: close\r\n\r\n{page}",
                page.len()
            );
            connection.write_all(response.as_bytes()).await.unwrap();
            connection.shutdown().await.unwrap();
        }
    });
    (origin, receipts, serving)
}
