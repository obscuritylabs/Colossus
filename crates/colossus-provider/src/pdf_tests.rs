use super::*;
use colossus_ports::{
    ResolvedRunInputFile, ResolvedRunInputImage, RunInputMediaError, RunInputMediaResolver,
};
use sha2::{Digest as _, Sha256};

const PDF: &[u8] = b"%PDF-1.4\nprivate-pdf-content\n%%EOF\n";

fn pdf_reference() -> ModelFileReference {
    ModelFileReference {
        artifact_id: format!("artifact-{}", "c".repeat(64)),
        file_name: "report.pdf".into(),
        media_type: "application/pdf".into(),
        size_bytes: PDF.len() as u64,
        sha256: format!("{:x}", Sha256::digest(PDF)),
    }
}

struct PdfResolver(AtomicUsize);

#[async_trait]
impl RunInputMediaResolver for PdfResolver {
    async fn resolve_image(
        &self,
        _: &ModelImageReference,
    ) -> Result<ResolvedRunInputImage, RunInputMediaError> {
        Err(RunInputMediaError::Unavailable)
    }
    async fn resolve_file(
        &self,
        reference: &ModelFileReference,
    ) -> Result<ResolvedRunInputFile, RunInputMediaError> {
        assert_eq!(reference, &pdf_reference());
        self.0.fetch_add(1, Ordering::AcqRel);
        Ok(ResolvedRunInputFile {
            reference: reference.clone(),
            bytes: PDF.to_vec(),
        })
    }
}

fn pdf_request(profile: &ProviderProfile, stream: bool) -> EffectRequest {
    let mut effect = provider_request(profile);
    effect.content["stream_response"] = json!(stream);
    let mut model = model_request_with_tools(&[]);
    model.messages[0].content = ModelContent::Parts(vec![
        ModelContentPart::Text {
            text: "before".into(),
        },
        ModelContentPart::File {
            file: pdf_reference(),
        },
        ModelContentPart::Text {
            text: "after".into(),
        },
    ]);
    effect.content["request"] = serde_json::to_value(model).expect("model content");
    effect
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let read = stream.read(&mut buffer).await.expect("request bytes");
        assert!(read > 0);
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("length"))
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8(bytes).expect("ASCII PDF fixture")
}

async fn pdf_server(
    kind: ProviderKind,
    stream: bool,
    failure: bool,
) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for index in 0..3 {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let request = read_request(&mut socket).await;
            let (status, content_type, body) = match index {
                0 => {
                    assert!(request.starts_with("POST /v1/files "));
                    assert!(request.contains("multipart/form-data; boundary="));
                    assert!(request.contains("name=\"purpose\"\r\n\r\nuser_data"));
                    assert!(request.contains("filename=\"report.pdf\""));
                    assert!(request.contains("private-pdf-content"));
                    (
                        200,
                        "application/json",
                        r#"{"id":"file-pdf-test"}"#.to_owned(),
                    )
                }
                1 => {
                    let payload: Value =
                        serde_json::from_str(request.split_once("\r\n\r\n").expect("body").1)
                            .expect("generation JSON");
                    if kind == ProviderKind::OpenAiResponses {
                        assert!(request.starts_with("POST /v1/responses "));
                        assert_eq!(
                            payload["input"][0]["content"][1],
                            json!({"type":"input_file", "file_id":"file-pdf-test"})
                        );
                    } else {
                        assert!(request.starts_with("POST /v1/chat/completions "));
                        assert_eq!(
                            payload["messages"][1]["content"][1],
                            json!({"type":"file", "file":{"file_id":"file-pdf-test"}})
                        );
                    }
                    assert!(!request.contains("private-pdf-content"));
                    if failure {
                        (
                            400,
                            "application/json",
                            r#"{"error":{"message":"file-pdf-test PDF unsupported"}}"#.to_owned(),
                        )
                    } else if stream && kind == ProviderKind::OpenAiResponses {
                        (200, "text/event-stream", "data: {\"type\":\"response.output_text.delta\",\"delta\":\"pdf answer\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-pdf\",\"status\":\"completed\",\"output\":[]}}\n\n".into())
                    } else if stream {
                        (200, "text/event-stream", "data: {\"id\":\"response-pdf\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"pdf answer\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into())
                    } else if kind == ProviderKind::OpenAiResponses {
                        (200, "application/json", json!({"id":"response-pdf", "output":[{"type":"message", "role":"assistant", "content":[{"type":"output_text", "text":"pdf answer"}]}]}).to_string())
                    } else {
                        (200, "application/json", json!({"id":"response-pdf", "choices":[{"message":{"role":"assistant","content":"pdf answer"}}]}).to_string())
                    }
                }
                _ => {
                    assert!(request.starts_with("DELETE /v1/files/file-pdf-test "));
                    (
                        200,
                        "application/json",
                        r#"{"id":"file-pdf-test","deleted":true}"#.to_owned(),
                    )
                }
            };
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer unit-secret")
            );
            requests.push(request);
            socket.write_all(format!("HTTP/1.1 {status} Result\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.expect("response");
        }
        requests
    });
    (format!("http://{address}/v1"), task)
}

async fn exercise_pdf(kind: ProviderKind, stream: bool, failure: bool, diagnostics: bool) {
    let (base, server) = pdf_server(kind, stream, failure).await;
    let profile = ProviderProfile::new(
        "pdf",
        kind,
        Some(base),
        Some("env:UNIT_PROVIDER_KEY".into()),
        5000,
    )
    .expect("profile");
    let resolver = Arc::new(PdfResolver(AtomicUsize::new(0)));
    let executor = ProviderExecutor::with_credentials(
        profile.clone(),
        Arc::new(CountingCredentialResolver::new()),
    )
    .with_run_input_media(resolver.clone());
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action(kind.generation_action(), DecisionOutcome::Allow)
        .with_network_destination(profile.network_origin().expect("origin").expect("network"))
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        journal.clone(),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [8; 32],
    );
    let mut effect = pdf_request(&profile, stream);
    effect.content["include_response_diagnostics"] = json!(diagnostics);
    let result = if stream {
        gateway
            .execute_stream(effect, &executor, &mut ReleasedItems::default())
            .await
            .map(|_| ())
    } else {
        gateway.execute(effect, &executor).await.map(|released| {
            if diagnostics {
                let diagnostic: ProviderResponseDiagnostic =
                    serde_json::from_slice(&released.bytes).expect("diagnostic");
                assert_eq!(diagnostic.status, 400);
                assert!(!String::from_utf8_lossy(&released.bytes).contains("file-pdf-test"));
                return;
            }
            let turn: ProviderTurn = serde_json::from_slice(&released.bytes).expect("turn");
            assert!(turn.events.iter().any(
                |event| matches!(event, ProviderEvent::FinalOutput { text } if text == "pdf answer")
            ));
        })
    };
    assert_eq!(result.is_err(), failure && !diagnostics, "{result:?}");
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("cleanup completed")
        .expect("server");
    assert_eq!(resolver.0.load(Ordering::Acquire), 1);
    let evidence = journal
        .read_global(1, 100)
        .expect("evidence")
        .iter()
        .map(|event| {
            journal
                .decrypt_payload(event)
                .expect("audit payload")
                .to_string()
        })
        .collect::<String>();
    assert!(!evidence.contains("private-pdf-content"));
    assert!(!evidence.contains("file-pdf-test"));
    assert!(!evidence.contains("unit-secret"));
}

#[tokio::test]
async fn pdf_upload_generation_and_cleanup_use_both_protocols_and_transports() {
    for kind in [
        ProviderKind::OpenAiResponses,
        ProviderKind::OpenAiCompatible,
    ] {
        for stream in [false, true] {
            exercise_pdf(kind, stream, false, false).await;
        }
        exercise_pdf(kind, false, true, false).await;
        exercise_pdf(kind, false, true, true).await;
        exercise_pdf(kind, true, true, true).await;
    }
}

#[tokio::test]
async fn denied_pdf_effect_never_resolves_or_uploads_private_bytes() {
    let profile = ProviderProfile::new(
        "pdf",
        ProviderKind::OpenAiResponses,
        Some("http://127.0.0.1:9/v1".into()),
        Some("env:UNIT_PROVIDER_KEY".into()),
        1000,
    )
    .expect("profile");
    let resolver = Arc::new(PdfResolver(AtomicUsize::new(0)));
    let credentials = Arc::new(CountingCredentialResolver::new());
    let executor = ProviderExecutor::with_credentials(profile.clone(), credentials.clone())
        .with_run_input_media(resolver.clone());
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [8; 32],
    );
    assert!(
        gateway
            .execute(pdf_request(&profile, false), &executor)
            .await
            .is_err()
    );
    assert_eq!(resolver.0.load(Ordering::Acquire), 0);
    assert_eq!(credentials.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn unsupported_pdf_provider_rejects_before_resolving_or_uploading() {
    let profile =
        ProviderProfile::new("pdf", ProviderKind::Echo, None, None, 1000).expect("profile");
    let resolver = Arc::new(PdfResolver(AtomicUsize::new(0)));
    let executor = ProviderExecutor::new(profile.clone()).with_run_input_media(resolver.clone());
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [8; 32],
    );
    let error = gateway
        .execute(pdf_request(&profile, false), &executor)
        .await
        .expect_err("Echo PDF rejection");
    assert!(error.to_string().contains("PDF inputs require"));
    assert_eq!(resolver.0.load(Ordering::Acquire), 0);
}

#[test]
fn pdf_diagnostics_redact_remote_file_ids_for_both_wire_formats() {
    let payload = json!({"input":[{"content":[{"type":"input_file","file_id":"file-private"}]}],"messages":[{"content":[{"type":"file","file":{"file_id":"file-private"}}]}]});
    let redacted = super::super::executor::redacted_image_payload(&payload).to_string();
    assert!(!redacted.contains("file-private"));
    assert!(redacted.contains("REDACTED_PROVIDER_FILE_ID"));
}

struct LivePdf {
    reference: ModelFileReference,
    bytes: Vec<u8>,
}

#[async_trait]
impl RunInputMediaResolver for LivePdf {
    async fn resolve_image(
        &self,
        _: &ModelImageReference,
    ) -> Result<ResolvedRunInputImage, RunInputMediaError> {
        Err(RunInputMediaError::Unavailable)
    }
    async fn resolve_file(
        &self,
        reference: &ModelFileReference,
    ) -> Result<ResolvedRunInputFile, RunInputMediaError> {
        if reference != &self.reference {
            return Err(RunInputMediaError::Unavailable);
        }
        Ok(ResolvedRunInputFile {
            reference: reference.clone(),
            bytes: self.bytes.clone(),
        })
    }
}

fn live_pdf(token: &str) -> LivePdf {
    let content = format!("BT /F1 18 Tf 72 720 Td ({token}) Tj ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
    let mut pdf = "%PDF-1.4\n".to_owned();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    let bytes = pdf.into_bytes();
    LivePdf {
        reference: ModelFileReference {
            artifact_id: format!("artifact-{}", "c".repeat(64)),
            file_name: "colossus-live.pdf".into(),
            media_type: "application/pdf".into(),
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        },
        bytes,
    }
}

#[tokio::test]
#[ignore = "requires a live PDF-capable provider, Files API, and an environment-backed credential"]
async fn live_pdf_inputs_extract_document_text_through_both_apis() {
    let base = std::env::var("COLOSSUS_LIVE_PDF_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".into());
    let model = std::env::var("COLOSSUS_LIVE_PDF_MODEL")
        .expect("set COLOSSUS_LIVE_PDF_MODEL to an accessible PDF-capable model");
    let credential = std::env::var("COLOSSUS_LIVE_PDF_CREDENTIAL_REFERENCE")
        .unwrap_or_else(|_| "env:OPENAI_API_KEY".into());
    let token = format!(
        "COLOSSUS-PDF-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let pdf = Arc::new(live_pdf(&token));
    for kind in [
        ProviderKind::OpenAiResponses,
        ProviderKind::OpenAiCompatible,
    ] {
        let profile = ProviderProfile::new(
            "live-pdf",
            kind,
            Some(base.clone()),
            Some(credential.clone()),
            120_000,
        )
        .expect("profile");
        let executor = ProviderExecutor::new(profile.clone()).with_run_input_media(pdf.clone());
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action(kind.generation_action(), DecisionOutcome::Allow)
                    .with_action_timeout(kind.generation_action(), 120_000)
                    .with_network_destination(
                        profile.network_origin().expect("origin").expect("network"),
                    )
                    .with_post_effect(true),
            ),
            Arc::new(DenyApproval),
            SafetyKernel::new(["provider.call".into()]),
            [8; 32],
        );
        for stream in [false, true] {
            let mut effect = pdf_request(&profile, stream);
            effect.content["model"] = json!(model);
            let mut request = model_request_with_tools(&[]);
            request.instructions =
                "Read the attached PDF. Reply with only its verification code.".into();
            request.messages[0].content = ModelContent::Parts(vec![ModelContentPart::File {
                file: pdf.reference.clone(),
            }]);
            effect.content["request"] = serde_json::to_value(request).expect("request");
            let output = if stream {
                let mut observer = ReleasedItems::default();
                gateway
                    .execute_stream(effect, &executor, &mut observer)
                    .await
                    .expect("live PDF stream");
                observer
                    .0
                    .iter()
                    .filter_map(|item| match item {
                        ProviderStreamItem::Event {
                            event: ProviderEvent::FinalOutput { text },
                        } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>()
            } else {
                let result = gateway
                    .execute(effect, &executor)
                    .await
                    .expect("live PDF turn");
                let turn: ProviderTurn = serde_json::from_slice(&result.bytes).expect("turn");
                turn.events
                    .iter()
                    .filter_map(|event| match event {
                        ProviderEvent::FinalOutput { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>()
            };
            assert!(
                output.contains(&token),
                "provider must extract the code available only inside the PDF"
            );
        }
    }
}
