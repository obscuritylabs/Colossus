use super::*;

pub(super) async fn client_handshake<S>(
    stream: &mut S,
    key: &[u8; 32],
) -> Result<String, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut challenge = [0_u8; 32];
    getrandom::fill(&mut challenge).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let challenge = hex::encode(challenge);
    write_message(
        stream,
        &ClientHello {
            version: PROTOCOL_VERSION,
            challenge: challenge.clone(),
        },
        1024,
    )
    .await?;
    let hello: ServerHello = read_message(stream, 1024).await?;
    if hello.version != PROTOCOL_VERSION
        || hello.challenge != challenge
        || hello.server_nonce.len() != 64
        || hex::decode(&hello.server_nonce).map_or(true, |bytes| bytes.len() != 32)
        || (now_ms() - hello.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "worker server protocol is incompatible or its handshake is invalid; restart the worker with this Colossus version".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedServerHello {
            version: hello.version,
            challenge: &hello.challenge,
            server_nonce: &hello.server_nonce,
            timestamp_ms: hello.timestamp_ms,
        },
        &hello.authentication_tag,
        "worker server handshake",
    )?;
    Ok(hello.server_nonce)
}

pub(super) async fn server_handshake<S>(
    stream: &mut S,
    key: &[u8; 32],
) -> Result<String, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello: ClientHello = read_message(stream, 1024).await?;
    if hello.version != PROTOCOL_VERSION
        || hello.challenge.len() != 64
        || hex::decode(&hello.challenge).map_or(true, |bytes| bytes.len() != 32)
    {
        return Err(WorkerError::Protocol(
            "worker client protocol is incompatible or its handshake is invalid; restart the worker and client with the same Colossus version".into(),
        ));
    }
    let mut server_nonce = [0_u8; 32];
    getrandom::fill(&mut server_nonce).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let server_nonce = hex::encode(server_nonce);
    let timestamp_ms = now_ms();
    let authentication_tag = request_tag(
        key,
        &UnsignedServerHello {
            version: PROTOCOL_VERSION,
            challenge: &hello.challenge,
            server_nonce: &server_nonce,
            timestamp_ms,
        },
    )?;
    write_message(
        stream,
        &ServerHello {
            version: PROTOCOL_VERSION,
            challenge: hello.challenge,
            server_nonce: server_nonce.clone(),
            timestamp_ms,
            authentication_tag,
        },
        1024,
    )
    .await?;
    Ok(server_nonce)
}
