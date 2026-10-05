//! Generate owner-private local TLS and OIDC configuration without production secrets.
use colossus_cloud::CloudPermission;
use colossus_cloud_server::config::{Config, Membership, OidcConfig, Storage};
use colossus_home::ColossusHome;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: init_local ABSOLUTE_PRIVATE_DIR")?,
    );
    let home = ColossusHome::ensure_at(path)?;
    let root = home.confined_root();
    if root.path().join("cloud.json").exists() {
        return Err("local cloud configuration already exists".into());
    }
    let ca_key = KeyPair::generate()?;
    let mut ca = CertificateParams::default();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let ca_cert = ca.self_signed(&ca_key)?;
    let issuer = Issuer::new(ca, ca_key);
    let server_key = KeyPair::generate()?;
    let mut server = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])?;
    server.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server.signed_by(&server_key, &issuer)?;
    for (name, bytes) in [
        ("ca.pem", ca_cert.pem()),
        ("ca-key.pem", issuer.key().serialize_pem()),
        ("server.pem", server_cert.pem()),
        ("server-key.pem", server_key.serialize_pem()),
    ] {
        let file = root.open_file(Path::new(name))?;
        file.file().write_all(bytes.as_bytes())?;
        file.file().sync_all()?;
    }
    let config = Config {
        http_bind: "127.0.0.1:8090".parse()?,
        grpc_bind: "127.0.0.1:8443".parse()?,
        public_origin: "http://127.0.0.1:5180".into(),
        grpc_endpoint: "https://localhost:8443".into(),
        ca_certificate: root.path().join("ca.pem"),
        ca_key: root.path().join("ca-key.pem"),
        server_certificate: root.path().join("server.pem"),
        server_key: root.path().join("server-key.pem"),
        oidc: OidcConfig {
            issuer: "http://127.0.0.1:8180/realms/colossus".into(),
            client_id: "colossus-cloud".into(),
            client_secret_file: None,
        },
        memberships: vec![Membership {
            subject: "11111111-1111-4111-8111-111111111111".into(),
            project_id: "local-project".into(),
            permissions: BTreeSet::from([
                CloudPermission::Read,
                CloudPermission::Execute,
                CloudPermission::Control,
                CloudPermission::Approve,
                CloudPermission::Administer,
            ]),
        }],
        web_root: std::env::current_dir()?.join("apps/web/dist"),
        storage: Storage::Redb {
            path: root.path().join("cloud.redb"),
            key_variable: None,
        },
        local_development: true,
        signing_key_variable: None,
    };
    let file = root.open_file(Path::new("cloud.json"))?;
    serde_json::to_writer_pretty(file.file(), &config)?;
    file.file().sync_all()?;
    root.sync_directory()?;
    println!(
        "Local cloud configuration is ready at {}",
        root.path().join("cloud.json").display()
    );
    Ok(())
}
