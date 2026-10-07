use crate::{CloudDatabaseConfig, CloudDatabaseTls};
use colossus_cloud::{CloudError, CloudResult};
use colossus_network::AdditionalRootCertificates;
use diesel::ConnectionResult;
use diesel_async::{AsyncPgConnection, SimpleAsyncConnection};
use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, pem::PemObject},
};
use std::{sync::Arc, time::Duration};
use tokio_postgres::{
    Config, NoTls,
    config::{Host, SslMode},
};
use tokio_postgres_rustls::MakeRustlsConnect;

fn categorical_connection_error() -> diesel::ConnectionError {
    diesel::ConnectionError::BadConnection("cloud PostgreSQL connection is unavailable".into())
}

pub(super) fn trust(
    config: &CloudDatabaseConfig,
    additional: &AdditionalRootCertificates,
) -> CloudResult<Option<MakeRustlsConnect>> {
    let mut roots = match &config.tls {
        CloudDatabaseTls::Disabled => return Ok(None),
        CloudDatabaseTls::WebpkiRoots => {
            let mut roots = RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            additional
                .add_to_rustls(&mut roots)
                .map_err(|_| CloudError::InvalidArgument)?;
            roots
        }
        CloudDatabaseTls::CustomCa { ca_pem_path } => {
            let bytes = std::fs::read(ca_pem_path).map_err(|_| CloudError::InvalidArgument)?;
            let mut roots = RootCertStore::empty();
            for cert in CertificateDer::pem_slice_iter(&bytes) {
                roots
                    .add(cert.map_err(|_| CloudError::InvalidArgument)?)
                    .map_err(|_| CloudError::InvalidArgument)?;
            }
            if roots.is_empty() {
                return Err(CloudError::InvalidArgument);
            }
            roots
        }
    };
    // The configured roots remain verified; no certificate/hostname bypass is exposed.
    roots.roots.shrink_to_fit();
    let builder =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|_| CloudError::InvalidArgument)?
            .with_root_certificates(roots);
    let tls = match additional.client_identity() {
        Some(identity) => {
            let (certs, key) = identity.rustls_material();
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|_| CloudError::InvalidArgument)?
        }
        None => builder.with_no_client_auth(),
    };
    Ok(Some(MakeRustlsConnect::new(tls)))
}

pub(super) fn parse(value: &str, config: &CloudDatabaseConfig) -> CloudResult<Config> {
    let mut pg = value
        .parse::<Config>()
        .map_err(|_| CloudError::InvalidArgument)?;
    pg.application_name("colossus-cloud");
    pg.connect_timeout(Duration::from_millis(config.connection_timeout_ms));
    if config.tls == CloudDatabaseTls::Disabled {
        if !pg.get_hosts().iter().all(|host| match host {
            Host::Tcp(host) => {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            }
            #[cfg(unix)]
            Host::Unix(_) => true,
        }) {
            return Err(CloudError::InvalidArgument);
        }
        pg.ssl_mode(SslMode::Disable);
    } else {
        pg.ssl_mode(SslMode::Require);
    }
    Ok(pg)
}

pub(super) async fn establish(
    value: &str,
    config: &CloudDatabaseConfig,
    tls: Option<MakeRustlsConnect>,
) -> ConnectionResult<AsyncPgConnection> {
    let pg = parse(value, config).map_err(|_| categorical_connection_error())?;
    let mut connection = match tls {
        Some(tls) => {
            let (client, connection) = pg
                .connect(tls)
                .await
                .map_err(|_| categorical_connection_error())?;
            AsyncPgConnection::try_from_client_and_connection(client, connection)
                .await
                .map_err(|_| categorical_connection_error())?
        }
        None => {
            let (client, connection) = pg
                .connect(NoTls)
                .await
                .map_err(|_| categorical_connection_error())?;
            AsyncPgConnection::try_from_client_and_connection(client, connection)
                .await
                .map_err(|_| categorical_connection_error())?
        }
    };
    connection
        .batch_execute(&format!(
            "SET search_path TO \"{}\"; SET statement_timeout = {}; SET lock_timeout = {}",
            config.schema, config.statement_timeout_ms, config.statement_timeout_ms
        ))
        .await
        .map_err(|_| categorical_connection_error())?;
    Ok(connection)
}
