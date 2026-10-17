//! Bounded categorical native startup evidence before the bridge ready handshake.
use std::time::Duration;

use tokio::{io::AsyncReadExt as _, net::UnixStream};

pub fn failure_category(error: &'static str, path: Option<&std::path::Path>) -> &'static str {
    use std::io::Read as _;
    let Some(file) = path.and_then(|path| std::fs::File::open(path).ok()) else {
        return error;
    };
    let mut bytes = Vec::new();
    if file.take(64 * 1024).read_to_end(&mut bytes).is_ok()
        && bytes
            .windows(b"close symbol missing".len())
            .any(|window| window == b"close symbol missing")
    {
        return "fixture native CEF sandbox close symbol missing";
    }
    error
}

pub async fn initialized(mut bootstrap: UnixStream) -> Result<(), &'static str> {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut previous = 0_u8;
        for _ in 0..8 {
            let mut record = [0_u8; 5];
            bootstrap
                .read_exact(&mut record)
                .await
                .map_err(|_| match previous {
                    0 => "fixture native disconnected before private descriptor proof",
                    1 => "fixture native disconnected while decoding configuration",
                    2 => "fixture native disconnected while preparing profile",
                    3 => "fixture native disconnected while preparing HOME",
                    4 => "fixture native disconnected while configuring proxy",
                    5 => "fixture native disconnected while preparing CEF options",
                    6 => "fixture native disconnected during CEF initialization",
                    _ => "fixture native startup receipt disconnected",
                })?;
            if record[..4] != [b'C', b'B', b'H', 1] {
                return Err("fixture native startup receipt invalid");
            }
            let next = record[4];
            if next & 0x80 != 0 {
                if next & 0x7f != previous {
                    return Err("fixture native startup receipt invalid");
                }
                return Err(match previous {
                    1 => "fixture native configuration failed",
                    2 => "fixture native profile failed",
                    3 => "fixture native HOME failed",
                    4 => "fixture native proxy failed",
                    5 => "fixture native CEF options failed",
                    6 => "fixture native CEF initialization failed",
                    _ => "fixture native startup failed",
                });
            }
            if next != previous + 1 || next > 7 {
                return Err("fixture native startup receipt invalid");
            }
            previous = next;
            if next == 7 {
                return Ok(());
            }
        }
        Err("fixture native startup receipt limit exceeded")
    })
    .await
    .map_err(|_| "fixture native initialization timed out")?
}
