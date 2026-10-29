use super::*;
use crate::wire::{ClientHello, ServerHello, now_ms, read_message, write_message};

#[tokio::test]
async fn previous_worker_protocol_is_rejected_before_any_operation_is_sent() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let peer = tokio::spawn(async move {
        let hello: ClientHello = read_message(&mut server, 1024).await.unwrap();
        assert_eq!(hello.version, PROTOCOL_VERSION);
        write_message(
            &mut server,
            &ServerHello {
                version: PROTOCOL_VERSION - 1,
                challenge: hello.challenge,
                server_nonce: hex::encode([9u8; 32]),
                timestamp_ms: now_ms(),
                authentication_tag: String::new(),
            },
            1024,
        )
        .await
        .unwrap();
        let mut byte = [0u8; 1];
        assert_eq!(
            tokio::io::AsyncReadExt::read(&mut server, &mut byte)
                .await
                .unwrap(),
            0,
            "an incompatible worker must not receive an authenticated operation"
        );
    });
    let result = client_handshake(&mut client, &[7u8; 32]).await;
    assert!(
        matches!(result,Err(WorkerControlError::Protocol(message)) if message.contains("protocol is incompatible"))
    );
    drop(client);
    peer.await.unwrap();
}
