use crate::{Bootstrap, BrokerLease, BrokerLimits, framing};
use base64::Engine as _;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    pki_types::{PrivateKeyDer, ServerName},
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_rustls::{TlsAcceptor, TlsConnector, client::TlsStream};

pub(super) const GENERATION: [u8; 32] = [42; 32];

pub(super) fn limits() -> BrokerLimits {
    BrokerLimits {
        connections: 8,
        lifetime: Duration::from_secs(30),
        exchange_timeout: Duration::from_secs(5),
        exchange_bytes: 1024 * 1024,
    }
}

pub(super) struct Fixture {
    pub listener: TcpListener,
    pub root: Vec<u8>,
    pub config: Arc<ServerConfig>,
}

impl Fixture {
    pub async fn new(host: &str, expired: bool) -> Self {
        Self::versions(host, expired, rustls::DEFAULT_VERSIONS).await
    }

    pub async fn versions(
        host: &str,
        expired: bool,
        versions: &[&'static rustls::SupportedProtocolVersion],
    ) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let ca_key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::new(params, ca_key);
        let mut leaf = CertificateParams::new(vec![host.to_owned()]).unwrap();
        leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        if expired {
            leaf.not_before = rcgen::date_time_ymd(2000, 1, 1);
            leaf.not_after = rcgen::date_time_ymd(2001, 1, 1);
        }
        let key = KeyPair::generate().unwrap();
        let cert = leaf.signed_by(&key, &issuer).unwrap();
        let mut config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(versions)
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![cert.der().clone()],
                    PrivateKeyDer::Pkcs8(key.serialize_der().into()),
                )
                .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Self {
            listener: TcpListener::bind(("127.0.0.1", 0)).await.unwrap(),
            root: ca.der().to_vec(),
            config: Arc::new(config),
        }
    }

    pub fn origin(&self) -> String {
        format!(
            "https://localhost:{}",
            self.listener.local_addr().unwrap().port()
        )
    }

    pub async fn broker(&self) -> BrokerLease {
        self.with_roots(vec![self.root.clone()]).await
    }

    pub async fn with_roots(&self, ca_der: Vec<Vec<u8>>) -> BrokerLease {
        BrokerLease::start(
            Bootstrap {
                origin: self.origin(),
                destinations: vec![self.listener.local_addr().unwrap()],
                ca_der,
                generation: GENERATION,
            },
            limits(),
        )
        .await
        .unwrap()
    }

    pub fn response(self, response: Vec<u8>) -> JoinHandle<Vec<u8>> {
        tokio::spawn(async move {
            let (socket, _) = self.listener.accept().await.unwrap();
            let mut stream = TlsAcceptor::from(self.config).accept(socket).await.unwrap();
            assert_eq!(
                stream.get_ref().1.alpn_protocol(),
                Some(b"http/1.1".as_slice())
            );
            let header = framing::head(&mut stream).await.unwrap();
            let body = framing::request_body(&mut stream, &header, limits().exchange_bytes)
                .await
                .unwrap();
            // Preserve what the real upstream received. Reusing the production
            // rewrite would hide a leaked proxy credential from this assertion.
            let mut observed = format!("{}\r\n", header.first).into_bytes();
            for (name, value) in &header.fields {
                observed.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
            }
            observed.extend_from_slice(b"\r\n");
            observed.extend_from_slice(&body);
            for bytes in response.chunks(137) {
                stream.write_all(bytes).await.unwrap();
            }
            stream.shutdown().await.unwrap();
            observed
        })
    }

    pub fn rejection(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let (socket, _) = self.listener.accept().await.unwrap();
            if let Ok(mut stream) = TlsAcceptor::from(self.config).accept(socket).await {
                let mut bytes = [0_u8; 1];
                assert!(!matches!(stream.read(&mut bytes).await, Ok(1)));
            }
        })
    }
}

pub(super) fn authorization(broker: &BrokerLease) -> String {
    let password = broker.credential(&GENERATION).unwrap();
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("colossus:{password}"))
    )
}

pub(super) async fn connect(
    broker: &BrokerLease,
    origin: &str,
    sni: &str,
) -> Result<TlsStream<TcpStream>, ()> {
    connect_versions(broker, origin, sni, rustls::DEFAULT_VERSIONS).await
}

pub(super) async fn connect_versions(
    broker: &BrokerLease,
    origin: &str,
    sni: &str,
    versions: &[&'static rustls::SupportedProtocolVersion],
) -> Result<TlsStream<TcpStream>, ()> {
    let url = url::Url::parse(origin).unwrap();
    let authority = format!(
        "{}:{}",
        url.host_str().unwrap(),
        url.port_or_known_default().unwrap()
    );
    let mut socket = TcpStream::connect(broker.address()).await.unwrap();
    socket.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: {}\r\n\r\n", authorization(broker)).as_bytes()).await.unwrap();
    let mut ready = [0_u8; 39];
    socket.read_exact(&mut ready).await.map_err(|_| ())?;
    assert_eq!(&ready, b"HTTP/1.1 200 Connection Established\r\n\r\n");
    let mut roots = RootCertStore::empty();
    roots
        .add(broker.public_ca(&GENERATION).unwrap().to_vec().into())
        .unwrap();
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(versions)
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(sni.to_owned()).unwrap(), socket)
        .await
        .map_err(|_| ())
}

pub(super) async fn exchange(broker: &BrokerLease, origin: &str, request: &[u8]) -> Vec<u8> {
    let mut client = connect(broker, origin, "localhost").await.unwrap();
    assert_eq!(
        client.get_ref().1.alpn_protocol(),
        Some(b"http/1.1".as_slice())
    );
    client.write_all(request).await.unwrap();
    let mut response = Vec::new();
    let _ = client.read_to_end(&mut response).await;
    response
}

pub(super) fn request(origin: &str) -> Vec<u8> {
    let host = origin.strip_prefix("https://").unwrap();
    format!("GET /verified HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n\r\n").into_bytes()
}
