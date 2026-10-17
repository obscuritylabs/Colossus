//! Strict bounded HTTP/1.1 framing; decrypted page bytes never become diagnostics.
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::{Zeroize as _, Zeroizing};

const MAX_HEAD: usize = 16 * 1024;
const MAX_REQUEST_BODY: usize = 1024 * 1024;

pub(crate) struct Head {
    pub first: String,
    pub fields: Vec<(String, String)>,
    pub initial: Zeroizing<Vec<u8>>,
}

impl Drop for Head {
    fn drop(&mut self) {
        self.first.zeroize();
        for (name, value) in &mut self.fields {
            name.zeroize();
            value.zeroize();
        }
    }
}

impl Head {
    pub fn one(&self, name: &str) -> Result<Option<&str>, ()> {
        let mut matches = self
            .fields
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name));
        let value = matches.next().map(|(_, value)| value.as_str());
        if matches.next().is_some() {
            return Err(());
        }
        Ok(value)
    }

    pub fn length(&self) -> Result<Option<usize>, ()> {
        self.one("content-length")?
            .map(|value| {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(());
                }
                value.parse().map_err(|_| ())
            })
            .transpose()
    }

    pub fn ordinary(&self) -> Result<(), ()> {
        for name in ["upgrade", "expect", "trailer", "te", "proxy-connection"] {
            if self.one(name)?.is_some() {
                return Err(());
            }
        }
        if self.one("connection")?.is_some_and(|value| {
            value.split(',').any(|token| {
                !matches!(
                    token.trim().to_ascii_lowercase().as_str(),
                    "close" | "keep-alive"
                )
            })
        }) {
            return Err(());
        }
        Ok(())
    }

    pub fn rewritten(&self, first: &str) -> Zeroizing<Vec<u8>> {
        let mut bytes = Zeroizing::new(first.as_bytes().to_vec());
        bytes.extend_from_slice(b"\r\n");
        for (name, value) in &self.fields {
            if [
                "connection",
                "proxy-authorization",
                "proxy-authenticate",
                "proxy-connection",
            ]
            .iter()
            .any(|excluded| name.eq_ignore_ascii_case(excluded))
            {
                continue;
            }
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(b": ");
            bytes.extend_from_slice(value.as_bytes());
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.extend_from_slice(b"Connection: close\r\n\r\n");
        bytes
    }
}

pub(crate) fn token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

pub(crate) async fn head<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Head, ()> {
    let mut captured = Zeroizing::new(Vec::new());
    let end = loop {
        if let Some(end) = captured.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break end + 4;
        }
        let mut buffer = [0_u8; 1024];
        let count = stream.read(&mut buffer).await.map_err(|_| ())?;
        if count == 0 || captured.len() + count > MAX_HEAD {
            return Err(());
        }
        captured.extend_from_slice(&buffer[..count]);
    };
    let parsed = std::str::from_utf8(&captured[..end - 4]).map_err(|_| ())?;
    let mut lines = parsed.split("\r\n");
    let first = lines.next().ok_or(())?;
    if !first.bytes().all(|byte| matches!(byte, 0x20..=0x7e)) {
        return Err(());
    }
    // Construct the wiping owner before collecting fields so malformed later
    // lines also erase already captured authorization/cookie values on error.
    let mut result = Head {
        first: first.to_owned(),
        fields: Vec::new(),
        initial: Zeroizing::new(captured[end..].to_vec()),
    };
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(())?;
        if !token(name)
            || !value
                .bytes()
                .all(|byte| matches!(byte, 0x20..=0x7e | b'\t'))
        {
            return Err(());
        }
        let value = value.trim();
        if value.is_empty() || result.fields.len() == 100 {
            return Err(());
        }
        result.fields.push((name.to_owned(), value.to_owned()));
    }
    Ok(result)
}

async fn exact<S: AsyncRead + Unpin>(
    stream: &mut S,
    initial: &[u8],
    count: usize,
) -> Result<Zeroizing<Vec<u8>>, ()> {
    if initial.len() > count {
        return Err(());
    }
    let mut bytes = Zeroizing::new(vec![0_u8; count]);
    bytes[..initial.len()].copy_from_slice(initial);
    stream
        .read_exact(&mut bytes[initial.len()..])
        .await
        .map_err(|_| ())?;
    Ok(bytes)
}

pub(crate) async fn request_body<S: AsyncRead + Unpin>(
    stream: &mut S,
    head: &Head,
    max: usize,
) -> Result<Zeroizing<Vec<u8>>, ()> {
    head.ordinary()?;
    if head.one("transfer-encoding")?.is_some() {
        return Err(());
    }
    let count = head.length()?.unwrap_or(0);
    if count > MAX_REQUEST_BODY || count > max {
        return Err(());
    }
    exact(stream, &head.initial, count).await
}

pub(crate) async fn response_body<S: AsyncRead + Unpin>(
    stream: &mut S,
    head: &Head,
    method: &str,
    max: usize,
) -> Result<Zeroizing<Vec<u8>>, ()> {
    head.ordinary()?;
    let parts: Vec<_> = head.first.split(' ').collect();
    if parts.len() < 2 || parts[0] != "HTTP/1.1" || parts[1].len() != 3 {
        return Err(());
    }
    let status: u16 = parts[1].parse().map_err(|_| ())?;
    if !(200..=599).contains(&status) {
        return Err(());
    }
    let length = head.length()?;
    let encoding = head.one("transfer-encoding")?;
    if length.is_some() && encoding.is_some() {
        return Err(());
    }
    if method == "HEAD" || matches!(status, 204 | 304) {
        if encoding.is_some() || (status == 204 && length.is_some_and(|length| length != 0)) {
            return Err(());
        }
        return exact(stream, &head.initial, 0).await;
    }
    match (length, encoding) {
        (Some(count), None) if count <= max => exact(stream, &head.initial, count).await,
        (None, Some("chunked")) => chunked(stream, &head.initial, max).await,
        _ => Err(()),
    }
}

async fn chunked<S: AsyncRead + Unpin>(
    stream: &mut S,
    initial: &[u8],
    max: usize,
) -> Result<Zeroizing<Vec<u8>>, ()> {
    if initial.len() > max {
        return Err(());
    }
    let mut bytes = Zeroizing::new(initial.to_vec());
    let mut cursor = 0;
    loop {
        let start = cursor;
        let end = loop {
            if let Some(offset) = bytes[cursor..]
                .windows(2)
                .position(|bytes| bytes == b"\r\n")
            {
                break cursor + offset;
            }
            if bytes.len() - start > 17 || bytes.len() == max {
                return Err(());
            }
            let byte = stream.read_u8().await.map_err(|_| ())?;
            bytes.push(byte);
        };
        let size = std::str::from_utf8(&bytes[start..end]).map_err(|_| ())?;
        if size.is_empty() || size.len() > 16 || !size.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(());
        }
        let size = usize::from_str_radix(size, 16).map_err(|_| ())?;
        cursor = end + 2;
        let required = cursor
            .checked_add(size)
            .and_then(|value| value.checked_add(2))
            .ok_or(())?;
        if required > max {
            return Err(());
        }
        if bytes.len() < required {
            let prior = bytes.len();
            bytes.resize(required, 0);
            stream
                .read_exact(&mut bytes[prior..])
                .await
                .map_err(|_| ())?;
        }
        if &bytes[required - 2..required] != b"\r\n" {
            return Err(());
        }
        cursor = required;
        if size == 0 {
            if cursor != bytes.len() {
                return Err(());
            }
            return Ok(bytes);
        }
    }
}
