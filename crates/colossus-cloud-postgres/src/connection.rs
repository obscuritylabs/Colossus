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
        CloudDatabaseTls::Disabled | CloudDatabaseTls::InCluster { .. } => return Ok(None),
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
    config.validate()?;
    let mut pg = value
        .parse::<Config>()
        .map_err(|_| CloudError::InvalidArgument)?;
    pg.application_name("colossus-cloud");
    pg.connect_timeout(Duration::from_millis(config.connection_timeout_ms));
    if let CloudDatabaseTls::InCluster { service, namespace } = &config.tls {
        let expected = format!("{service}.{namespace}.svc");
        if !matches!(pg.get_hosts(), [Host::Tcp(host)] if host == &expected)
            || !pg.get_hostaddrs().is_empty()
        {
            return Err(CloudError::InvalidArgument);
        }
        pg.ssl_mode(SslMode::Disable);
    } else if config.tls == CloudDatabaseTls::Disabled {
        if !pg.get_hosts().iter().all(|host| match host {
            Host::Tcp(host) => {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            }
            #[cfg(unix)]
            Host::Unix(_) => true,
        }) || !pg.get_hostaddrs().iter().all(std::net::IpAddr::is_loopback)
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(tls: CloudDatabaseTls) -> CloudDatabaseConfig {
        CloudDatabaseConfig {
            connection_variable: "DATABASE_URL".into(),
            schema: "control_plane".into(),
            tls,
            max_connections: 8,
            connection_timeout_ms: 5000,
            statement_timeout_ms: 15000,
        }
    }

    #[test]
    fn in_cluster_plaintext_requires_one_exact_service_and_no_address_override() {
        let config = config(CloudDatabaseTls::InCluster {
            service: "colossus-postgres".into(),
            namespace: "colossus".into(),
        });
        let parsed = parse("host=colossus-postgres.colossus.svc user=colossus", &config).unwrap();
        assert_eq!(parsed.get_ssl_mode(), SslMode::Disable);
        for value in [
            "host=database.example.com",
            "host=colossus-postgres.other.svc",
            "host=colossus-postgres",
            "host=colossus-postgres.colossus.svc.evil.example",
            "host=colossus-postgres.colossus.svc,external.example.com",
            "host=colossus-postgres.colossus.svc hostaddr=203.0.113.1",
            "hostaddr=127.0.0.1",
            "host=/tmp",
            "user=colossus",
        ] {
            assert!(parse(value, &config).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn disabled_fixture_tls_cannot_override_loopback_with_a_remote_address() {
        let config = config(CloudDatabaseTls::Disabled);
        assert!(parse("host=localhost hostaddr=203.0.113.1", &config).is_err());
        assert!(parse("host=localhost hostaddr=127.0.0.1", &config).is_ok());
    }

    #[test]
    fn verified_tls_is_required_even_when_the_url_requests_plaintext() {
        let config = config(CloudDatabaseTls::WebpkiRoots);
        let parsed = parse("host=database.example.com sslmode=disable", &config).unwrap();
        assert_eq!(parsed.get_ssl_mode(), SslMode::Require);
    }

    #[test]
    fn in_cluster_names_are_dns_labels_and_roundtrip_as_explicit_configuration() {
        for name in [
            "",
            "other.svc",
            "-postgres",
            "postgres-",
            "Postgres",
            "../postgres",
        ] {
            assert!(
                config(CloudDatabaseTls::InCluster {
                    service: name.into(),
                    namespace: "colossus".into(),
                })
                .validate()
                .is_err()
            );
            assert!(
                config(CloudDatabaseTls::InCluster {
                    service: "postgres".into(),
                    namespace: name.into(),
                })
                .validate()
                .is_err()
            );
        }
        let value = config(CloudDatabaseTls::InCluster {
            service: "postgres".into(),
            namespace: "colossus".into(),
        });
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(json["tls"]["kind"], "in_cluster");
        let decoded: CloudDatabaseConfig = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, value);
    }
}
