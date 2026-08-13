use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_network::AdditionalRootCertificates;
use futures_util::StreamExt as _;
use minisign_verify::{PublicKey, Signature};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::{Update, UpdaterExt as _};

use crate::{
    desktop_dto::{DesktopReleaseChannelDto, DesktopUpdateCheckDto},
    desktop_settings::SettingsStore,
    dto::CommandErrorDto,
    state::AppState,
};

const UPDATE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_UPDATE_BYTES: usize = 512 * 1024 * 1024;
const MAX_UPDATE_SIGNATURE_BYTES: usize = 16 * 1024;
const UPDATE_ENDPOINT: &str = env!("COLOSSUS_DESKTOP_UPDATE_ENDPOINT");
const UPDATE_PUBLIC_KEY: &str = env!("COLOSSUS_DESKTOP_UPDATE_PUBLIC_KEY");

#[tauri::command]
pub(crate) async fn check_desktop_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DesktopUpdateCheckDto, CommandErrorDto> {
    let _guard = state.try_update_guard().ok_or_else(update_busy)?;
    let current_version = app.package_info().version.to_string();
    if !updates_configured() {
        state.set_update_available(false);
        return Ok(DesktopUpdateCheckDto {
            configured: false,
            available: false,
            current_version,
            version: None,
            channel: DesktopReleaseChannelDto::current(),
        });
    }
    let update = checked_update(&app).await?;
    let version = update.as_ref().map(|candidate| candidate.version.clone());
    let available = update.is_some();
    state.set_update_available(available);
    Ok(DesktopUpdateCheckDto {
        configured: true,
        available,
        current_version,
        version,
        channel: DesktopReleaseChannelDto::current(),
    })
}

#[tauri::command]
pub(crate) async fn install_desktop_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, CommandErrorDto> {
    let _guard = state.try_update_guard().ok_or_else(update_busy)?;
    if !updates_configured() {
        return Err(update_unavailable());
    }
    let Some(update) = checked_update(&app).await? else {
        state.set_update_available(false);
        return Ok(false);
    };
    let version = update.version.clone();
    let app_for_dialog = app.clone();
    let approved = tauri::async_runtime::spawn_blocking(move || {
        app_for_dialog
            .dialog()
            .message(format!(
                "Download, verify, and install Colossus Desktop {version} from the configured {} update channel?",
                channel_name()
            ))
            .title("Install Colossus Desktop update")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Install update".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| update_error(true))?;
    if !approved {
        return Ok(false);
    }

    let package = download_verified_update(&update, additional_roots()?).await?;
    update.install(package).map_err(|_| update_error(false))?;
    state.set_update_available(false);
    Ok(true)
}

async fn download_verified_update(
    update: &Update,
    roots: AdditionalRootCertificates,
) -> Result<Vec<u8>, CommandErrorDto> {
    let client = configure_update_client(reqwest::Client::builder(), &roots)
        .timeout(UPDATE_TIMEOUT)
        .build()
        .map_err(|_| update_error(true))?;
    let response = client
        .get(update.download_url.clone())
        .send()
        .await
        .map_err(|_| update_error(true))?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_UPDATE_BYTES as u64)
    {
        return Err(update_error(true));
    }
    let mut package = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| update_error(true))?;
        if package.len().saturating_add(chunk.len()) > MAX_UPDATE_BYTES {
            return Err(update_error(false));
        }
        package.extend_from_slice(&chunk);
    }
    verify_update_signature(&package, &update.signature, UPDATE_PUBLIC_KEY)?;
    Ok(package)
}

fn verify_update_signature(
    package: &[u8],
    encoded_signature: &str,
    encoded_public_key: &str,
) -> Result<(), CommandErrorDto> {
    if encoded_signature.len() > MAX_UPDATE_SIGNATURE_BYTES
        || encoded_public_key.len() > MAX_UPDATE_SIGNATURE_BYTES
    {
        return Err(update_configuration_error());
    }
    let public_key = STANDARD
        .decode(encoded_public_key)
        .ok()
        .and_then(|value| String::from_utf8(value).ok())
        .and_then(|value| PublicKey::decode(&value).ok())
        .ok_or_else(update_configuration_error)?;
    let signature = STANDARD
        .decode(encoded_signature)
        .ok()
        .and_then(|value| String::from_utf8(value).ok())
        .and_then(|value| Signature::decode(&value).ok())
        .ok_or_else(update_configuration_error)?;
    public_key
        .verify(package, &signature, true)
        .map_err(|_| update_error(false))
}

async fn checked_update(app: &AppHandle) -> Result<Option<Update>, CommandErrorDto> {
    let endpoint = UPDATE_ENDPOINT
        .parse()
        .map_err(|_| update_configuration_error())?;
    let roots = additional_roots()?;
    let expected_channel = channel_name();
    let expected_target = update_target();
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|_| update_configuration_error())?
        .target(expected_target)
        .pubkey(UPDATE_PUBLIC_KEY)
        .timeout(UPDATE_TIMEOUT)
        .configure_client(move |builder| configure_update_client(builder, &roots))
        .build()
        .map_err(|_| update_configuration_error())?;
    let update = updater.check().await.map_err(|_| update_error(true))?;
    let Some(update) = update else {
        return Ok(None);
    };
    if !valid_update_metadata(
        update.download_url.scheme(),
        &update.raw_json,
        expected_channel,
    ) {
        return Err(update_configuration_error());
    }
    Ok(Some(update))
}

fn configure_update_client(
    builder: reqwest::ClientBuilder,
    roots: &AdditionalRootCertificates,
) -> reqwest::ClientBuilder {
    roots
        .configure_reqwest(builder)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() == "https" {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
}

fn valid_update_metadata(
    download_scheme: &str,
    raw_json: &serde_json::Value,
    expected_channel: &str,
) -> bool {
    download_scheme == "https"
        && raw_json
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            == Some(1)
        && raw_json.get("channel").and_then(serde_json::Value::as_str) == Some(expected_channel)
}

fn additional_roots() -> Result<AdditionalRootCertificates, CommandErrorDto> {
    let store = SettingsStore::open_application()?;
    let settings = store.load()?;
    let Some(bundle) = settings.additional_ca_bundle.as_ref() else {
        return Ok(AdditionalRootCertificates::default());
    };
    AdditionalRootCertificates::from_pem_bundle_path(store.ca_bundle_path(bundle)?)
        .map_err(|_| update_configuration_error())
}

fn updates_configured() -> bool {
    !UPDATE_ENDPOINT.is_empty()
        && !UPDATE_PUBLIC_KEY.is_empty()
        && matches!(
            DesktopReleaseChannelDto::current(),
            DesktopReleaseChannelDto::Stable | DesktopReleaseChannelDto::DeveloperPreview
        )
}

fn update_target() -> String {
    format!(
        "{}-{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        channel_name()
    )
}

fn channel_name() -> &'static str {
    match DesktopReleaseChannelDto::current() {
        DesktopReleaseChannelDto::Stable => "stable",
        DesktopReleaseChannelDto::DeveloperPreview => "developer_preview",
        DesktopReleaseChannelDto::Development => "development",
        DesktopReleaseChannelDto::ValidationOnly => "validation_only",
    }
}

fn update_busy() -> CommandErrorDto {
    CommandErrorDto::busy("A Desktop update check or installation is already in progress.")
}

fn update_unavailable() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "update_unavailable",
        "This Desktop build does not advertise an update channel.",
        false,
    )
}

fn update_configuration_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "update_configuration",
        "The Desktop update channel failed its security validation.",
        false,
    )
}

fn update_error(retryable: bool) -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "update_failed",
        "The Desktop update could not be checked, verified, or installed.",
        retryable,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa,
        KeyPair,
    };
    use rustls::{
        ServerConfig,
        pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
    };
    use std::sync::Arc;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };
    use tokio_rustls::TlsAcceptor;

    #[test]
    fn development_build_has_no_update_authority() {
        if DesktopReleaseChannelDto::current() == DesktopReleaseChannelDto::Development {
            assert!(!updates_configured());
            assert_eq!(channel_name(), "development");
        }
    }

    #[test]
    fn update_target_is_channel_and_platform_scoped() {
        let target = update_target();
        assert!(target.contains(std::env::consts::OS));
        assert!(target.contains(std::env::consts::ARCH));
        assert!(target.ends_with(channel_name()));
    }

    #[test]
    fn metadata_must_match_channel_and_secure_transport() {
        let metadata = serde_json::json!({
            "schemaVersion": 1,
            "channel": "developer_preview"
        });
        assert!(valid_update_metadata(
            "https",
            &metadata,
            "developer_preview"
        ));
        assert!(!valid_update_metadata(
            "http",
            &metadata,
            "developer_preview"
        ));
        assert!(!valid_update_metadata("https", &metadata, "stable"));
        assert!(!valid_update_metadata(
            "https",
            &serde_json::json!({"schemaVersion": 2, "channel": "developer_preview"}),
            "developer_preview"
        ));
    }

    #[test]
    fn updater_errors_do_not_expose_endpoints_or_paths() {
        for error in [
            update_unavailable(),
            update_configuration_error(),
            update_error(true),
        ] {
            let serialized = serde_json::to_string(&error).expect("serialize error");
            assert!(!serialized.contains("https://"));
            assert!(!serialized.contains("/Users/"));
            assert!(!serialized.contains("\\\\"));
        }
    }

    #[test]
    fn malformed_signing_material_fails_closed() {
        let error = verify_update_signature(b"package", "bad", "bad")
            .expect_err("invalid update signing material");
        assert_eq!(error.code, "update_configuration");
        assert!(!error.retryable);
    }

    #[test]
    fn known_minisign_package_signature_verifies() {
        let public_key = "untrusted comment: minisign public key E7620F1842B4E81F\n\
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        let signature = "untrusted comment: signature from minisign secret key\n\
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\n\
trusted comment: timestamp:1555779966\tfile:test\n\
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        verify_update_signature(
            b"test",
            &STANDARD.encode(signature),
            &STANDARD.encode(public_key),
        )
        .expect("known minisign fixture verifies");
        assert!(
            verify_update_signature(
                b"tampered",
                &STANDARD.encode(signature),
                &STANDARD.encode(public_key)
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn update_client_accepts_an_imported_private_ca() {
        let mut ca_params =
            CertificateParams::new(vec!["Colossus Update Test CA".into()]).expect("CA params");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().expect("CA key"))
            .expect("CA certificate");
        let mut server_params =
            CertificateParams::new(vec!["127.0.0.1".into()]).expect("server params");
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_key = KeyPair::generate().expect("server key");
        let server_certificate = server_params
            .signed_by(&server_key, &ca)
            .expect("server certificate");
        let roots = AdditionalRootCertificates::from_pem_bundle(ca.pem().as_bytes())
            .expect("test CA roots");
        let server_config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("TLS protocol versions")
                .with_no_client_auth()
                .with_single_cert(
                    vec![server_certificate.der().clone()],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
                )
                .expect("TLS server");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut stream = TlsAcceptor::from(Arc::new(server_config))
                .accept(stream)
                .await
                .expect("TLS");
            let mut request = [0_u8; 4_096];
            let read = stream.read(&mut request).await.expect("request");
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /update"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 7\r\nconnection: close\r\n\r\npackage",
                )
                .await
                .expect("response");
        });
        let client = configure_update_client(reqwest::Client::builder(), &roots)
            .build()
            .expect("update client");
        let body = client
            .get(format!("https://{address}/update"))
            .send()
            .await
            .expect("private CA request")
            .bytes()
            .await
            .expect("body");
        assert_eq!(body.as_ref(), b"package");
        server.await.expect("server task");
    }
}
