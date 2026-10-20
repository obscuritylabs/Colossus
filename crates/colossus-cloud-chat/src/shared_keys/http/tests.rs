use super::*;
use std::{io::Write, net::TcpListener};

fn response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn fixture(
    response: String,
    update: Option<SharedAnchorUpdate>,
) -> Result<SharedAnchorSnapshot, AuthorityError> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            let count = socket.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.contains(&format!("/v1/anchors/{}", "a".repeat(64))));
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer authority-fixture-token")
        );
        socket.write_all(response.as_bytes()).unwrap();
    });
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let (reply, _) = mpsc::sync_channel(1);
    let request = Request {
        namespace: "a".repeat(64),
        update,
        deadline: Instant::now() + CALL_TIMEOUT,
        reply,
    };
    let result = exchange(
        &client,
        &Url::parse(&format!("http://{address}/v1/anchors")).unwrap(),
        &HeaderValue::from_static("Bearer authority-fixture-token"),
        &request,
    );
    server.join().unwrap();
    result
}

#[test]
fn authority_read_accepts_empty_attestation_and_rejects_invalid_envelopes() {
    let empty = r#"{"version":1,"revision":0,"anchor":null}"#;
    assert_eq!(
        fixture(response("200 OK", empty), None).unwrap().revision,
        0
    );
    for body in [
        r#"{"version":2,"revision":0,"anchor":null}"#,
        r#"{"version":1,"revision":2,"anchor":null}"#,
        r#"{"version":1,"revision":0,"anchor":null,"unexpected":"value"}"#,
    ] {
        assert!(matches!(
            fixture(response("200 OK", body), None),
            Err(AuthorityError::Unavailable)
        ));
    }
}

#[test]
fn authority_errors_and_oversize_responses_fail_closed() {
    for status in [
        "401 Unauthorized",
        "409 Conflict",
        "503 Service Unavailable",
        "302 Found",
    ] {
        assert!(matches!(
            fixture(response(status, "{}"), None),
            Err(AuthorityError::Unavailable)
        ));
    }
    assert!(matches!(
        fixture(
            response("200 OK", &" ".repeat(RESPONSE_LIMIT as usize + 1)),
            None
        ),
        Err(AuthorityError::Unavailable)
    ));
    let update = SharedAnchorUpdate {
        version: 1,
        expected_revision: 0,
        anchor: SecureAnchor {
            format_version: 2,
            sequence: 0,
            hash: "0".repeat(64),
            verification_profile: None,
            status: colossus_contracts::SecureAnchorStatus::Verified,
        },
    };
    assert!(matches!(
        fixture(response("409 Conflict", "{}"), Some(update)),
        Err(AuthorityError::Conflict)
    ));
}
