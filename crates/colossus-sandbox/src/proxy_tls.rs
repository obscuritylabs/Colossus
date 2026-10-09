//! Bounded TLS ClientHello capture and SNI validation for CONNECT tunnels.

use super::*;

pub(super) async fn read_tls_client_hello(
    client: &mut TcpStream,
    initial: &[u8],
) -> Result<Vec<u8>, ExecutionError> {
    let mut captured = initial.to_vec();
    let mut handshake = Vec::new();
    let mut offset = 0_usize;
    loop {
        read_proxy_bytes(client, &mut captured, offset.saturating_add(5)).await?;
        if captured[offset] != 22 {
            return Err(adapter_failure(
                "CONNECT tunnel did not begin with a TLS handshake record",
            ));
        }
        let record_len = usize::from(u16::from_be_bytes([
            captured[offset + 3],
            captured[offset + 4],
        ]));
        if record_len == 0 || record_len > MAX_TLS_RECORD_BYTES {
            return Err(adapter_failure("TLS handshake record is oversized"));
        }
        let record_end = offset.saturating_add(5).saturating_add(record_len);
        read_proxy_bytes(client, &mut captured, record_end).await?;
        handshake.extend_from_slice(&captured[offset + 5..record_end]);
        if handshake.len() > MAX_TLS_CLIENT_HELLO_BYTES {
            return Err(adapter_failure("TLS ClientHello is oversized"));
        }
        if handshake.len() >= 4 {
            if handshake[0] != 1 {
                return Err(adapter_failure(
                    "CONNECT tunnel did not begin with a TLS ClientHello",
                ));
            }
            let hello_len = (usize::from(handshake[1]) << 16)
                | (usize::from(handshake[2]) << 8)
                | usize::from(handshake[3]);
            if hello_len > MAX_TLS_CLIENT_HELLO_BYTES.saturating_sub(4) {
                return Err(adapter_failure("TLS ClientHello is oversized"));
            }
            if handshake.len() >= hello_len.saturating_add(4) {
                return Ok(captured);
            }
        }
        offset = record_end;
    }
}

pub(super) async fn read_proxy_bytes(
    client: &mut TcpStream,
    captured: &mut Vec<u8>,
    required: usize,
) -> Result<(), ExecutionError> {
    while captured.len() < required {
        if required > MAX_TLS_CLIENT_HELLO_BYTES.saturating_add(MAX_TLS_RECORD_BYTES) {
            return Err(adapter_failure("TLS ClientHello is oversized"));
        }
        let mut buffer = [0_u8; 4096];
        let count = client.read(&mut buffer).await.map_err(adapter_failure)?;
        if count == 0 {
            return Err(adapter_failure("TLS ClientHello ended unexpectedly"));
        }
        captured.extend_from_slice(&buffer[..count]);
    }
    Ok(())
}

pub(super) fn tls_server_name(
    client_hello_records: &[u8],
) -> Result<Option<String>, ExecutionError> {
    let mut handshake = Vec::new();
    let mut offset = 0_usize;
    while offset.saturating_add(5) <= client_hello_records.len() {
        if client_hello_records[offset] != 22 {
            break;
        }
        let record_len = usize::from(u16::from_be_bytes([
            client_hello_records[offset + 3],
            client_hello_records[offset + 4],
        ]));
        let record_end = offset.saturating_add(5).saturating_add(record_len);
        if record_end > client_hello_records.len() {
            return Err(adapter_failure("TLS ClientHello record is truncated"));
        }
        handshake.extend_from_slice(&client_hello_records[offset + 5..record_end]);
        if handshake.len() >= 4 {
            let hello_len = (usize::from(handshake[1]) << 16)
                | (usize::from(handshake[2]) << 8)
                | usize::from(handshake[3]);
            if handshake.len() >= hello_len.saturating_add(4) {
                break;
            }
        }
        offset = record_end;
    }
    let hello_len = tls_u24(&handshake, 1)?;
    if handshake.first() != Some(&1) || handshake.len() < hello_len.saturating_add(4) {
        return Err(adapter_failure("TLS ClientHello is invalid"));
    }
    let body = &handshake[4..4 + hello_len];
    let mut cursor = 34;
    cursor = skip_tls_vector(body, cursor, 1)?;
    cursor = skip_tls_vector(body, cursor, 2)?;
    cursor = skip_tls_vector(body, cursor, 1)?;
    if cursor == body.len() {
        return Ok(None);
    }
    let extensions_len = tls_u16(body, cursor)?;
    cursor = cursor.saturating_add(2);
    let extensions_end = cursor.saturating_add(extensions_len);
    if extensions_end != body.len() {
        return Err(adapter_failure("TLS ClientHello extensions are invalid"));
    }
    while cursor < extensions_end {
        let extension_type = tls_u16(body, cursor)?;
        let extension_len = tls_u16(body, cursor.saturating_add(2))?;
        cursor = cursor.saturating_add(4);
        let extension_end = cursor.saturating_add(extension_len);
        if extension_end > extensions_end {
            return Err(adapter_failure("TLS ClientHello extension is truncated"));
        }
        if extension_type == 0 {
            let names_len = tls_u16(body, cursor)?;
            let mut name_cursor = cursor.saturating_add(2);
            if name_cursor.saturating_add(names_len) != extension_end {
                return Err(adapter_failure("TLS server-name extension is invalid"));
            }
            while name_cursor < extension_end {
                let name_type = *body
                    .get(name_cursor)
                    .ok_or_else(|| adapter_failure("TLS server name is truncated"))?;
                let name_len = tls_u16(body, name_cursor.saturating_add(1))?;
                name_cursor = name_cursor.saturating_add(3);
                let name_end = name_cursor.saturating_add(name_len);
                if name_end > extension_end {
                    return Err(adapter_failure("TLS server name is truncated"));
                }
                if name_type == 0 {
                    let name = std::str::from_utf8(&body[name_cursor..name_end])
                        .map_err(adapter_failure)?;
                    if name.is_empty() || !name.is_ascii() {
                        return Err(adapter_failure("TLS server name is invalid"));
                    }
                    return Ok(Some(name.to_owned()));
                }
                name_cursor = name_end;
            }
            return Ok(None);
        }
        cursor = extension_end;
    }
    Ok(None)
}

pub(super) fn tls_u16(bytes: &[u8], offset: usize) -> Result<usize, ExecutionError> {
    let high = *bytes
        .get(offset)
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let low = *bytes
        .get(offset.saturating_add(1))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    Ok((usize::from(high) << 8) | usize::from(low))
}

pub(super) fn tls_u24(bytes: &[u8], offset: usize) -> Result<usize, ExecutionError> {
    let first = *bytes
        .get(offset)
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let second = *bytes
        .get(offset.saturating_add(1))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let third = *bytes
        .get(offset.saturating_add(2))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    Ok((usize::from(first) << 16) | (usize::from(second) << 8) | usize::from(third))
}

pub(super) fn skip_tls_vector(
    bytes: &[u8],
    offset: usize,
    length_bytes: usize,
) -> Result<usize, ExecutionError> {
    let length = match length_bytes {
        1 => usize::from(
            *bytes
                .get(offset)
                .ok_or_else(|| adapter_failure("TLS vector is truncated"))?,
        ),
        2 => tls_u16(bytes, offset)?,
        _ => return Err(adapter_failure("TLS vector length is unsupported")),
    };
    let end = offset.saturating_add(length_bytes).saturating_add(length);
    if end > bytes.len() {
        return Err(adapter_failure("TLS vector is truncated"));
    }
    Ok(end)
}
