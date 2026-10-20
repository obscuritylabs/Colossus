//! Actual CEF certificate rejection and the production sanitized log boundary.
use colossus_contracts::{BrowserAction, BrowserTarget, BrowserUrl};
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt as _, net::TcpListener, task::JoinHandle};
use tokio_rustls::TlsAcceptor;
use tracing::{Event, Metadata, Subscriber, field::Visit, span};

#[derive(Clone)]
pub(super) struct Target {
    pub origin: String,
    connections: Arc<AtomicUsize>,
    http_bytes: Arc<AtomicUsize>,
    evidence: Arc<Evidence>,
}

pub(super) struct Fixture {
    pub target: Target,
    task: JoinHandle<()>,
}

impl Fixture {
    pub async fn start() -> Self {
        let evidence = collector();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("private TLS fixture bind");
        let origin = format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        );
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .expect("unique self-signed native fixture certificate");
        let configuration = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("native fixture TLS versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der())
                .into(),
        )
        .expect("private TLS fixture configuration");
        let acceptor = TlsAcceptor::from(Arc::new(configuration));
        let target = Target {
            origin,
            connections: Arc::new(AtomicUsize::new(0)),
            http_bytes: Arc::new(AtomicUsize::new(0)),
            evidence,
        };
        let observed = target.clone();
        let task = tokio::spawn(async move {
            // Native retry/parallel connections remain bounded and owned. Dropping
            // the fixture aborts this listener and every in-progress handshake.
            let mut handshakes = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let Ok((socket, _)) = connection else { break };
                        if handshakes.len() >= 8 { drop(socket); continue; }
                        observed.connections.fetch_add(1, Ordering::SeqCst);
                        let acceptor = acceptor.clone();
                        let http_bytes = Arc::clone(&observed.http_bytes);
                        handshakes.spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(3), async {
                                if let Ok(mut connection) = acceptor.accept(socket).await {
                                    let mut bytes = [0; 1024];
                                    if let Ok(count) = connection.read(&mut bytes).await {
                                        http_bytes.fetch_add(count, Ordering::SeqCst);
                                    }
                                }
                            }).await;
                        });
                    }
                    _ = handshakes.join_next(), if !handshakes.is_empty() => {}
                }
            }
        });
        Self { target, task }
    }

    pub async fn finish(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn check(
    fixture: &Target,
    driver: &dyn BrowserDriver,
    request: &BrowserDriverOpenRequest,
    target: &BrowserTarget,
    control: &BrowserDriverControl,
) {
    let before = fixture.evidence.accepted.load(Ordering::SeqCst);
    let mut command = super::exercise::command(
        request,
        target,
        BrowserAction::Navigate {
            url: BrowserUrl::parse(format!("{}/untrusted", fixture.origin)).unwrap(),
        },
        960,
    );
    command.run_id = "next-native-run".into();
    command.control_generation = 2;
    let result = driver.execute(command, control).await;
    assert!(
        matches!(result, Err(BrowserDriverError::Failed)),
        "CEF must reject an admitted untrusted TLS server before HTTP"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while fixture.evidence.accepted.load(Ordering::SeqCst) == before {
        assert!(
            Instant::now() < deadline,
            "safe native TLS diagnostic missing"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!fixture.evidence.invalid.load(Ordering::SeqCst));
    assert!(fixture.connections.load(Ordering::SeqCst) > 0);
    assert_eq!(fixture.http_bytes.load(Ordering::SeqCst), 0);
    eprintln!(
        "PASS actual CEF rejects untrusted HTTPS before HTTP and emits only safe TLS diagnostics"
    );
}

#[derive(Default)]
struct Evidence {
    accepted: AtomicUsize,
    invalid: AtomicBool,
}

fn collector() -> Arc<Evidence> {
    static EVIDENCE: OnceLock<Arc<Evidence>> = OnceLock::new();
    Arc::clone(EVIDENCE.get_or_init(|| {
        let evidence = Arc::new(Evidence::default());
        tracing::subscriber::set_global_default(Collector(Arc::clone(&evidence)))
            .expect("native fixture requires its isolated operational log collector");
        evidence
    }))
}

struct Collector(Arc<Evidence>);

impl Subscriber for Collector {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "colossus.browser"
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut receipt = Receipt::default();
        event.record(&mut receipt);
        if !receipt.tls {
            return;
        }
        let fields = event.metadata().fields();
        let expected = [
            "event_name",
            "cef_error",
            "certificate_status",
            "system_chain_errors",
            "system_policy_error",
            "system_verification",
            "message",
        ];
        if receipt.cef_error != Some(-202)
            || receipt
                .certificate_status
                .is_none_or(|status| status & 4 == 0)
            || receipt.system_chain.is_none_or(|status| status == 0)
            || receipt.system_policy.is_none_or(|status| status == 0)
            || !receipt.untrusted
            || fields.len() != expected.len()
            || expected.iter().any(|name| fields.field(name).is_none())
        {
            self.0.invalid.store(true, Ordering::SeqCst);
        }
        self.0.accepted.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Receipt {
    tls: bool,
    untrusted: bool,
    cef_error: Option<i64>,
    certificate_status: Option<u64>,
    system_chain: Option<u64>,
    system_policy: Option<u64>,
}

impl Visit for Receipt {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "event_name" => self.tls = value == "browser.tls_verification_failed",
            "system_verification" => self.untrusted = value == "untrusted",
            _ => {}
        }
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        if field.name() == "cef_error" {
            self.cef_error = Some(value);
        }
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "certificate_status" => self.certificate_status = Some(value),
            "system_chain_errors" => self.system_chain = Some(value),
            "system_policy_error" => self.system_policy = Some(value),
            _ => {}
        }
    }
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}
