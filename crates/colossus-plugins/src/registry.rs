use super::*;

/// Signature enforcement selected by one reusable trust profile.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginTrustMode {
    /// A matching in-process Sigstore/Cosign verification is required.
    #[default]
    Required,
    /// Unmatched or unsigned content may be installed but remains untrusted.
    Optional,
    /// Enforce OCI digest integrity only.
    Disabled,
}

/// One keyless Sigstore identity accepted by a trust profile.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SigstoreIdentity {
    /// Exact certificate issuer.
    pub issuer: String,
    /// Exact certificate subject.
    pub subject: String,
}

/// Reusable supply-chain trust policy.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginTrustProfile {
    /// Required, optional, or explicitly disabled signature verification.
    pub mode: PluginTrustMode,
    /// PEM public key paths used for Cosign verification.
    pub public_keys: Vec<PathBuf>,
    /// Accepted keyless issuer/subject bindings.
    pub identities: Vec<SigstoreIdentity>,
    /// Optional local Sigstore trust-root bundle for disconnected verification.
    pub trust_root_path: Option<PathBuf>,
}

/// OCI registry credential source.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RegistryAuthConfig {
    /// Make unauthenticated OCI requests.
    #[default]
    Anonymous,
    /// Inject an environment-backed Bearer token.
    Bearer {
        /// Credential reference resolved only inside the authorized transport boundary.
        credential_reference: String,
    },
    /// Use a literal username and environment-backed password.
    Basic {
        /// Literal registry username.
        username: String,
        /// Password credential reference resolved only inside the transport boundary.
        credential_reference: String,
    },
    /// Read Docker auth only because this profile explicitly requests it.
    Docker {
        /// Optional explicit config path; omission selects Docker's platform default.
        #[serde(default)]
        config_path: Option<PathBuf>,
        /// Exact helper executables keyed by Docker helper suffix.
        #[serde(default)]
        helper_executables: BTreeMap<String, PathBuf>,
    },
}

/// Exact-origin OCI registry policy.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginRegistryProfile {
    /// Exact HTTPS origin, with loopback HTTP permitted for development.
    pub origin: String,
    /// Explicit authentication mechanism.
    pub auth: RegistryAuthConfig,
    /// Reusable trust profile name.
    pub trust_profile: String,
    /// Exact permitted Bearer token-service origins.
    pub token_origins: Vec<String>,
    /// Exact permitted cross-origin blob redirect origins.
    pub blob_redirect_origins: Vec<String>,
    /// Optional CA roots used only for the registry origin.
    pub ca_bundle_path: Option<PathBuf>,
    /// Optional CA roots keyed by exact allowed token-service origin.
    pub token_ca_bundle_paths: BTreeMap<String, PathBuf>,
    /// Optional CA roots keyed by exact allowed blob-redirect origin.
    pub blob_redirect_ca_bundle_paths: BTreeMap<String, PathBuf>,
    /// Permit an explicitly configured private or loopback registry network.
    pub allow_non_public: bool,
}

/// Parse a registry reference into its exact origin, repository, and tag or digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryReference {
    /// Canonical registry origin inferred from the reference host.
    pub origin: String,
    /// Slash-separated OCI repository name.
    pub repository: String,
    /// Explicit tag or `sha256:` digest selector.
    pub selector: String,
    /// Whether the reference already pins an immutable digest.
    pub digest_pinned: bool,
}

impl RegistryReference {
    /// Parse `registry.example/repository:tag` or `registry.example/repository@sha256:...`.
    pub fn parse(value: &str) -> Result<Self, StoreError> {
        if value.contains("//") || value.contains(['?', '#']) {
            return Err(StoreError::Adapter(
                "OCI references use registry/repository:tag syntax without a URL scheme".into(),
            ));
        }
        let (host, remainder) = value
            .split_once('/')
            .ok_or_else(|| StoreError::Adapter("OCI reference requires a repository".into()))?;
        if host.is_empty() || remainder.is_empty() {
            return Err(StoreError::Adapter("invalid OCI reference".into()));
        }
        let (repository, selector, digest_pinned) = if let Some((repository, digest)) =
            remainder.rsplit_once('@')
        {
            if !digest.starts_with("sha256:") {
                return Err(StoreError::Adapter(
                    "only sha256 OCI digests are supported".into(),
                ));
            }
            (repository, digest, true)
        } else {
            let slash = remainder.rfind('/').unwrap_or(0);
            let colon = remainder[slash..]
                .rfind(':')
                .map(|index| slash + index)
                .ok_or_else(|| {
                    StoreError::Adapter("OCI reference requires an explicit tag or digest".into())
                })?;
            (&remainder[..colon], &remainder[colon + 1..], false)
        };
        if repository.is_empty() || selector.is_empty() {
            return Err(StoreError::Adapter(
                "invalid OCI repository or selector".into(),
            ));
        }
        let scheme = if host.starts_with("localhost")
            || host.starts_with("127.")
            || host.starts_with("[::1]")
        {
            "http"
        } else {
            "https"
        };
        Ok(Self {
            origin: format!("{scheme}://{host}"),
            repository: repository.into(),
            selector: selector.into(),
            digest_pinned,
        })
    }
}

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_network::{AdditionalRootCertificates, pinned_reqwest_client};
use colossus_ports::CredentialResolver;
use futures::StreamExt as _;
use reqwest::{Method, Response, StatusCode, header};
use std::time::Duration;

#[path = "registry_upload.rs"]
mod upload;

const REGISTRY_TIMEOUT_MS: u64 = 60_000;
const MAX_REGISTRY_REDIRECTS: usize = 3;
const MAX_TOKEN_BYTES: u64 = 1024 * 1024;

/// Secret-bearing credential material resolved only for one authorized transfer.
#[derive(Clone, Eq, PartialEq)]
pub enum RegistryCredential {
    /// No preemptive credentials. A public Bearer challenge may still be followed.
    Anonymous,
    /// Pre-resolved Bearer token.
    Bearer(String),
    /// Pre-resolved HTTP Basic username and password.
    Basic {
        /// Literal registry username from the selected profile or Docker credential result.
        username: String,
        /// Secret registry password or token resolved immediately before transfer.
        password: String,
    },
}

impl std::fmt::Debug for RegistryCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Anonymous => "Anonymous",
            Self::Bearer(_) => "Bearer([REDACTED])",
            Self::Basic { .. } => "Basic([REDACTED])",
        })
    }
}

/// Result of resolving registry authentication without running subprocesses implicitly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryCredentialResolution {
    /// Authentication is ready for the registry transport.
    Ready(RegistryCredential),
    /// An exact configured Docker credential helper must run through the caller's process gateway.
    DockerHelper {
        /// Exact helper executable configured for the helper suffix.
        executable: PathBuf,
        /// Registry server value written to helper standard input.
        server: String,
    },
}

/// Bounded result of an OCI registry transfer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginRegistryTransfer {
    /// Credential-free OCI reference used for the transfer.
    pub reference: String,
    /// Canonical digest calculated from the fetched or pushed manifest bytes.
    pub manifest_digest: String,
    /// Total verified manifest, config, and layer bytes transferred.
    pub bytes: u64,
}

#[derive(Clone, Debug)]
struct BearerChallenge {
    realm: Url,
    service: Option<String>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(alias = "access_token")]
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerConfigFile {
    #[serde(default)]
    auths: BTreeMap<String, DockerAuthEntry>,
    #[serde(default)]
    cred_helpers: BTreeMap<String, String>,
    #[serde(default)]
    creds_store: Option<String>,
}

#[derive(Deserialize)]
struct DockerAuthEntry {
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    identitytoken: Option<String>,
}

/// Resolve anonymous, Bearer, Basic, or explicitly selected Docker `auths` credentials.
///
/// Docker credential helpers intentionally return an actionable error here; callers must run the
/// exact configured helper through their normal subprocess permit/audit boundary and construct
/// a `RegistryCredential` from that bounded result.
pub fn resolve_registry_credential(
    profile: &PluginRegistryProfile,
    credentials: &dyn CredentialResolver,
) -> Result<RegistryCredential, StoreError> {
    match resolve_registry_credential_source(profile, credentials)? {
        RegistryCredentialResolution::Ready(credential) => Ok(credential),
        RegistryCredentialResolution::DockerHelper { executable, server } => {
            Err(StoreError::Adapter(format!(
                "Docker credential helper {} must be run through the subprocess permit boundary for server {server}",
                executable.display()
            )))
        }
    }
}

/// Resolve registry authentication up to a possible explicit Docker helper subprocess.
pub fn resolve_registry_credential_source(
    profile: &PluginRegistryProfile,
    credentials: &dyn CredentialResolver,
) -> Result<RegistryCredentialResolution, StoreError> {
    match &profile.auth {
        RegistryAuthConfig::Anonymous => Ok(RegistryCredentialResolution::Ready(
            RegistryCredential::Anonymous,
        )),
        RegistryAuthConfig::Bearer {
            credential_reference,
        } => credentials
            .resolve(credential_reference)
            .map(RegistryCredential::Bearer)
            .map(RegistryCredentialResolution::Ready)
            .map_err(adapter),
        RegistryAuthConfig::Basic {
            username,
            credential_reference,
        } => credentials
            .resolve(credential_reference)
            .map(|password| RegistryCredential::Basic {
                username: username.clone(),
                password,
            })
            .map(RegistryCredentialResolution::Ready)
            .map_err(adapter),
        RegistryAuthConfig::Docker {
            config_path,
            helper_executables,
        } => resolve_docker_auths(
            config_path.as_deref(),
            helper_executables,
            &canonical_origin(&profile.origin)?,
        ),
    }
}

fn resolve_docker_auths(
    configured_path: Option<&Path>,
    helper_executables: &BTreeMap<String, PathBuf>,
    registry_origin: &str,
) -> Result<RegistryCredentialResolution, StoreError> {
    let path = configured_path.map(Path::to_owned).map_or_else(
        || {
            std::env::var_os("DOCKER_CONFIG")
                .map(PathBuf::from)
                .map(|path| path.join("config.json"))
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .map(|path| path.join(".docker/config.json"))
                })
                .ok_or_else(|| StoreError::Adapter("Docker config path is unavailable".into()))
        },
        Ok,
    )?;
    let config: DockerConfigFile =
        serde_json::from_slice(&read_bounded(&path, MAX_MANIFEST_BYTES)?).map_err(adapter)?;
    let url = Url::parse(registry_origin).map_err(adapter)?;
    let host = url
        .host_str()
        .ok_or_else(|| StoreError::Adapter("registry origin has no host".into()))?;
    let server = if let Some(port) = url.port() {
        format!("{host}:{port}")
    } else {
        host.into()
    };
    let helper = config
        .cred_helpers
        .get(&server)
        .or_else(|| config.cred_helpers.get(registry_origin))
        .or(config.creds_store.as_ref());
    if let Some(helper) = helper {
        let executable = helper_executables.get(helper).ok_or_else(|| {
            StoreError::Adapter(format!(
                "Docker credential helper {helper} has no exact configured executable"
            ))
        })?;
        return Ok(RegistryCredentialResolution::DockerHelper {
            executable: executable.clone(),
            server,
        });
    }
    let entry = config
        .auths
        .get(&server)
        .or_else(|| config.auths.get(registry_origin))
        .or_else(|| config.auths.get(&format!("https://{server}/v1/")))
        .ok_or_else(|| StoreError::Adapter("Docker config has no matching registry auth".into()))?;
    if let Some(token) = entry
        .identitytoken
        .as_ref()
        .filter(|token| !token.is_empty())
    {
        return Ok(RegistryCredentialResolution::Ready(
            RegistryCredential::Bearer(token.clone()),
        ));
    }
    let encoded = entry
        .auth
        .as_deref()
        .ok_or_else(|| StoreError::Adapter("Docker auth entry has no credential".into()))?;
    let decoded = BASE64
        .decode(encoded)
        .map_err(|_| StoreError::Adapter("Docker auth entry is not valid base64".into()))?;
    let decoded = String::from_utf8(decoded)
        .map_err(|_| StoreError::Adapter("Docker auth entry is not UTF-8".into()))?;
    let (username, password) = decoded
        .split_once(':')
        .ok_or_else(|| StoreError::Adapter("Docker auth entry has no username separator".into()))?;
    let credential = RegistryCredential::Basic {
        username: username.into(),
        password: password.into(),
    };
    validate_registry_credential(&credential)?;
    Ok(RegistryCredentialResolution::Ready(credential))
}

/// Return an exact configured Docker helper invocation without executing it.
///
/// `None` means the explicitly selected Docker configuration uses an `auths` entry and can be
/// resolved inside the registry effect itself.
pub fn docker_credential_helper(
    profile: &PluginRegistryProfile,
) -> Result<Option<(PathBuf, String)>, StoreError> {
    let RegistryAuthConfig::Docker {
        config_path,
        helper_executables,
    } = &profile.auth
    else {
        return Ok(None);
    };
    let path = docker_config_path(config_path.as_deref())?;
    let config: DockerConfigFile =
        serde_json::from_slice(&read_bounded(&path, MAX_MANIFEST_BYTES)?).map_err(adapter)?;
    let server = docker_registry_server(&canonical_origin(&profile.origin)?)?;
    let helper = config
        .cred_helpers
        .get(&server)
        .or_else(|| config.cred_helpers.get(&profile.origin))
        .or(config.creds_store.as_ref());
    helper
        .map(|helper| {
            helper_executables
                .get(helper)
                .cloned()
                .map(|executable| (executable, server.clone()))
                .ok_or_else(|| {
                    StoreError::Adapter(format!(
                        "Docker credential helper {helper} has no exact configured executable"
                    ))
                })
        })
        .transpose()
}

fn docker_config_path(configured_path: Option<&Path>) -> Result<PathBuf, StoreError> {
    configured_path.map(Path::to_owned).map_or_else(
        || {
            std::env::var_os("DOCKER_CONFIG")
                .map(PathBuf::from)
                .map(|path| path.join("config.json"))
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .map(|path| path.join(".docker/config.json"))
                })
                .ok_or_else(|| StoreError::Adapter("Docker config path is unavailable".into()))
        },
        Ok,
    )
}

fn docker_registry_server(registry_origin: &str) -> Result<String, StoreError> {
    let url = Url::parse(registry_origin).map_err(adapter)?;
    let host = url
        .host_str()
        .ok_or_else(|| StoreError::Adapter("registry origin has no host".into()))?;
    Ok(if let Some(port) = url.port() {
        format!("{host}:{port}")
    } else {
        host.into()
    })
}

/// Parse the bounded JSON emitted by a Docker credential helper `get` operation.
pub fn registry_credential_from_helper_output(
    bytes: &[u8],
) -> Result<RegistryCredential, StoreError> {
    if bytes.len() > 64 * 1024 {
        return Err(StoreError::Adapter(
            "Docker credential helper output exceeds 64 KiB".into(),
        ));
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase", deny_unknown_fields)]
    struct HelperOutput {
        username: String,
        secret: String,
    }
    let output: HelperOutput = serde_json::from_slice(bytes).map_err(|_| {
        StoreError::Adapter("Docker credential helper returned invalid JSON".into())
    })?;
    let credential = if output.username == "<token>" {
        RegistryCredential::Bearer(output.secret)
    } else {
        RegistryCredential::Basic {
            username: output.username,
            password: output.secret,
        }
    };
    validate_registry_credential(&credential)?;
    Ok(credential)
}

/// Hardened OCI Distribution v2 client for whole Agent Plugin artifacts.
pub struct PluginRegistryClient {
    profile: PluginRegistryProfile,
    credential: RegistryCredential,
    timeout_ms: u64,
    clients: tokio::sync::Mutex<BTreeMap<String, reqwest::Client>>,
}

impl PluginRegistryClient {
    /// Construct a registry client from an exact-origin profile and already resolved secret.
    pub fn new(
        profile: PluginRegistryProfile,
        credential: RegistryCredential,
    ) -> Result<Self, StoreError> {
        validate_registry_profile(&profile)?;
        validate_registry_credential(&credential)?;
        Ok(Self {
            profile,
            credential,
            timeout_ms: REGISTRY_TIMEOUT_MS,
            clients: tokio::sync::Mutex::new(BTreeMap::new()),
        })
    }

    /// Override the bounded end-to-end request timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout_ms = u64::try_from(timeout.as_millis())
            .unwrap_or(u64::MAX)
            .clamp(1, 300_000);
        self
    }

    /// Pull exactly one manifest, config, and content layer into a fresh OCI layout.
    pub async fn pull(
        &self,
        reference: &str,
        destination: &Path,
    ) -> Result<PluginRegistryTransfer, StoreError> {
        if destination.exists() {
            return Err(StoreError::Adapter(format!(
                "OCI layout destination already exists: {}",
                destination.display()
            )));
        }
        let parsed = self.reference(reference)?;
        let manifest_url = self.registry_url(&parsed, "manifests", &parsed.selector)?;
        let response = self
            .authenticated_get(
                manifest_url,
                Some(format!("repository:{}:pull", parsed.repository)),
                false,
            )
            .await?;
        require_status(&response, StatusCode::OK, "manifest pull")?;
        require_media_type(&response, OCI_IMAGE_MANIFEST_MEDIA_TYPE, "manifest")?;
        let declared_digest = response
            .headers()
            .get("docker-content-digest")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let manifest = read_response(response, MAX_MANIFEST_BYTES).await?;
        let manifest_digest = sha256_digest(&manifest);
        if parsed.digest_pinned && parsed.selector != manifest_digest {
            return Err(StoreError::Verification(
                "registry manifest does not match the requested immutable digest".into(),
            ));
        }
        if declared_digest
            .as_deref()
            .is_some_and(|digest| digest != manifest_digest)
        {
            return Err(StoreError::Verification(
                "registry manifest digest header does not match the response bytes".into(),
            ));
        }
        let parsed_manifest: colossus_contracts::AgentPluginOciManifest =
            serde_json::from_slice(&manifest).map_err(adapter)?;
        validate_plugin_oci_manifest(&parsed_manifest)?;

        let config = self
            .pull_blob(
                &parsed,
                &parsed_manifest.config.digest,
                parsed_manifest.config.size,
            )
            .await?;
        let layer_descriptor = parsed_manifest
            .layers
            .first()
            .ok_or_else(|| StoreError::Adapter("plugin OCI layer is missing".into()))?;
        let layer = self
            .pull_blob(&parsed, &layer_descriptor.digest, layer_descriptor.size)
            .await?;
        let (referrer_descriptors, referrer_blobs) = self
            .pull_referrer_material(&parsed, &manifest_digest)
            .await?;

        fs::create_dir_all(destination.join("blobs/sha256")).map_err(adapter)?;
        write_new(
            &destination.join("oci-layout"),
            br#"{"imageLayoutVersion":"1.0.0"}"#,
        )?;
        write_blob(destination, &config)?;
        write_blob(destination, &layer)?;
        write_blob(destination, &manifest)?;
        for blob in &referrer_blobs {
            write_blob(destination, blob)?;
        }
        let descriptor = colossus_contracts::OciDescriptor {
            media_type: OCI_IMAGE_MANIFEST_MEDIA_TYPE.into(),
            digest: manifest_digest.clone(),
            size: u64::try_from(manifest.len()).map_err(adapter)?,
            annotations: BTreeMap::from([(
                "org.opencontainers.image.ref.name".into(),
                parsed.selector.clone(),
            )]),
        };
        let mut descriptors = vec![serde_json::to_value(descriptor).map_err(adapter)?];
        descriptors.extend(referrer_descriptors);
        write_new(
            &destination.join("index.json"),
            &serde_json::to_vec(&json!({
                "schemaVersion": 2,
                "mediaType": OCI_IMAGE_INDEX_MEDIA_TYPE,
                "manifests": descriptors,
            }))
            .map_err(adapter)?,
        )?;
        let artifact = verify_plugin_layout(destination, Some(&manifest_digest));
        if let Err(error) = artifact {
            let _ = remove_tree_if_present(destination);
            return Err(error);
        }
        Ok(PluginRegistryTransfer {
            reference: reference.into(),
            manifest_digest,
            bytes: u64::try_from(manifest.len()).map_err(adapter)?
                + u64::try_from(config.len()).map_err(adapter)?
                + u64::try_from(layer.len()).map_err(adapter)?
                + referrer_blobs.iter().try_fold(0_u64, |total, blob| {
                    total
                        .checked_add(u64::try_from(blob.len()).map_err(adapter)?)
                        .ok_or_else(|| StoreError::Adapter("registry byte count overflow".into()))
                })?,
        })
    }

    /// Push one verified OCI layout using resumable blob uploads followed by manifest PUT.
    pub async fn push(
        &self,
        layout: &Path,
        reference: &str,
    ) -> Result<PluginRegistryTransfer, StoreError> {
        let parsed = self.reference(reference)?;
        let artifact = verify_plugin_layout(layout, None)?;
        if parsed.digest_pinned && parsed.selector != artifact.manifest_digest {
            return Err(StoreError::Verification(
                "local manifest does not match the requested immutable push digest".into(),
            ));
        }
        self.push_blob(
            &parsed,
            &artifact.parsed_manifest.config.digest,
            &artifact.config,
        )
        .await?;
        self.push_blob(
            &parsed,
            &artifact.parsed_manifest.layers[0].digest,
            &artifact.layer,
        )
        .await?;
        let url = self.registry_url(&parsed, "manifests", &parsed.selector)?;
        let response = self
            .authenticated_send(
                Method::PUT,
                url,
                Some(format!("repository:{}:push", parsed.repository)),
                Some(OCI_IMAGE_MANIFEST_MEDIA_TYPE),
                artifact.manifest.clone(),
            )
            .await?;
        if !matches!(
            response.status(),
            StatusCode::CREATED | StatusCode::ACCEPTED
        ) {
            return Err(registry_status("manifest push", response.status()));
        }
        if let Some(remote) = response
            .headers()
            .get("docker-content-digest")
            .and_then(|value| value.to_str().ok())
            && remote != artifact.manifest_digest
        {
            return Err(StoreError::Verification(
                "registry returned a different manifest digest".into(),
            ));
        }
        let referrer_bytes = self
            .push_referrer_material(layout, &parsed, &artifact.manifest_digest)
            .await?;
        Ok(PluginRegistryTransfer {
            reference: reference.into(),
            manifest_digest: artifact.manifest_digest,
            bytes: u64::try_from(artifact.manifest.len()).map_err(adapter)?
                + u64::try_from(artifact.config.len()).map_err(adapter)?
                + u64::try_from(artifact.layer.len()).map_err(adapter)?
                + referrer_bytes,
        })
    }

    /// Fetch the OCI 1.1 referrers index for an exact subject digest.
    pub async fn referrers(
        &self,
        reference: &str,
        subject_digest: &str,
    ) -> Result<Value, StoreError> {
        validate_digest(subject_digest)?;
        let parsed = self.reference(reference)?;
        let url = self.registry_url(&parsed, "referrers", subject_digest)?;
        let response = self
            .authenticated_get(
                url,
                Some(format!("repository:{}:pull", parsed.repository)),
                false,
            )
            .await?;
        require_status(&response, StatusCode::OK, "referrers pull")?;
        serde_json::from_slice(&read_response(response, MAX_MANIFEST_BYTES).await?).map_err(adapter)
    }

    fn reference(&self, value: &str) -> Result<RegistryReference, StoreError> {
        let reference = RegistryReference::parse(value)?;
        if canonical_origin(&reference.origin)? != canonical_origin(&self.profile.origin)? {
            return Err(StoreError::Adapter(
                "OCI reference origin does not match the selected registry profile".into(),
            ));
        }
        Ok(reference)
    }

    fn registry_url(
        &self,
        reference: &RegistryReference,
        family: &str,
        value: &str,
    ) -> Result<Url, StoreError> {
        validate_repository(&reference.repository)?;
        if family == "blobs" || family == "referrers" {
            validate_digest(value)?;
        } else {
            validate_selector(value)?;
        }
        Url::parse(&format!(
            "{}/v2/{}/{family}/{value}",
            self.profile.origin.trim_end_matches('/'),
            reference.repository
        ))
        .map_err(adapter)
    }

    async fn pull_blob(
        &self,
        reference: &RegistryReference,
        digest: &str,
        expected_size: u64,
    ) -> Result<Vec<u8>, StoreError> {
        validate_digest(digest)?;
        if expected_size > MAX_TOTAL_BYTES {
            return Err(StoreError::Adapter(
                "OCI blob exceeds the configured bound".into(),
            ));
        }
        let url = self.registry_url(reference, "blobs", digest)?;
        let response = self
            .authenticated_get(
                url,
                Some(format!("repository:{}:pull", reference.repository)),
                true,
            )
            .await?;
        require_status(&response, StatusCode::OK, "blob pull")?;
        let bytes = read_response(response, expected_size.min(MAX_TOTAL_BYTES)).await?;
        if u64::try_from(bytes.len()).map_err(adapter)? != expected_size
            || sha256_digest(&bytes) != digest
        {
            return Err(StoreError::Verification(
                "OCI blob size or digest does not match its descriptor".into(),
            ));
        }
        Ok(bytes)
    }

    async fn push_blob(
        &self,
        reference: &RegistryReference,
        digest: &str,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        validate_digest(digest)?;
        if sha256_digest(bytes) != digest {
            return Err(StoreError::Verification(
                "local OCI blob does not match its descriptor".into(),
            ));
        }
        let blob = self.registry_url(reference, "blobs", digest)?;
        let head = self
            .authenticated_head(
                blob,
                Some(format!("repository:{}:pull,push", reference.repository)),
            )
            .await?;
        if head.status() == StatusCode::OK {
            return Ok(());
        }
        if head.status() != StatusCode::NOT_FOUND {
            return Err(registry_status("blob existence check", head.status()));
        }
        let location = self.start_upload(reference).await?;
        let location = self.upload_content(reference, location, bytes).await?;
        let mut complete = location;
        complete.query_pairs_mut().append_pair("digest", digest);
        let response = self
            .authenticated_send(
                Method::PUT,
                complete,
                Some(format!("repository:{}:pull,push", reference.repository)),
                Some("application/octet-stream"),
                Vec::new(),
            )
            .await?;
        if response.status() != StatusCode::CREATED {
            return Err(registry_status("blob upload completion", response.status()));
        }
        Ok(())
    }

    async fn pull_referrer_material(
        &self,
        reference: &RegistryReference,
        subject_digest: &str,
    ) -> Result<(Vec<Value>, Vec<Vec<u8>>), StoreError> {
        let url = self.registry_url(reference, "referrers", subject_digest)?;
        let response = self
            .authenticated_get(
                url,
                Some(format!("repository:{}:pull", reference.repository)),
                false,
            )
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok((Vec::new(), Vec::new()));
        }
        require_status(&response, StatusCode::OK, "referrers pull")?;
        let index: Value =
            serde_json::from_slice(&read_response(response, MAX_MANIFEST_BYTES).await?)
                .map_err(adapter)?;
        if index.get("schemaVersion") != Some(&json!(2)) {
            return Err(StoreError::Adapter("invalid OCI referrers index".into()));
        }
        let descriptors = index
            .get("manifests")
            .and_then(Value::as_array)
            .ok_or_else(|| StoreError::Adapter("OCI referrers index has no manifests".into()))?;
        if descriptors.len() > 256 {
            return Err(StoreError::Adapter(
                "OCI referrers index exceeds 256 entries".into(),
            ));
        }
        let mut retained = Vec::new();
        let mut blobs = Vec::new();
        for descriptor in descriptors {
            let digest = descriptor
                .get("digest")
                .and_then(Value::as_str)
                .ok_or_else(|| StoreError::Adapter("referrer digest is absent".into()))?;
            let size = descriptor
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| StoreError::Adapter("referrer size is absent".into()))?;
            if size > MAX_MANIFEST_BYTES {
                return Err(StoreError::Adapter(
                    "OCI referrer manifest exceeds 1 MiB".into(),
                ));
            }
            if descriptor.get("mediaType").and_then(Value::as_str)
                != Some(OCI_IMAGE_MANIFEST_MEDIA_TYPE)
            {
                continue;
            }
            let manifest = self.pull_blob(reference, digest, size).await?;
            let value: Value = serde_json::from_slice(&manifest).map_err(adapter)?;
            if value
                .get("subject")
                .and_then(|subject| subject.get("digest"))
                .and_then(Value::as_str)
                != Some(subject_digest)
            {
                return Err(StoreError::Verification(
                    "OCI referrer subject does not match the plugin manifest".into(),
                ));
            }
            let mut children = Vec::new();
            if let Some(config) = value.get("config") {
                children.push(config);
            }
            if let Some(layers) = value.get("layers").and_then(Value::as_array) {
                children.extend(layers);
            }
            for child in children {
                let child_digest = child
                    .get("digest")
                    .and_then(Value::as_str)
                    .ok_or_else(|| StoreError::Adapter("referrer blob digest is absent".into()))?;
                let child_size = child
                    .get("size")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| StoreError::Adapter("referrer blob size is absent".into()))?;
                if child_size > MAX_FILE_BYTES {
                    return Err(StoreError::Adapter(
                        "OCI referrer blob exceeds 256 MiB".into(),
                    ));
                }
                blobs.push(self.pull_blob(reference, child_digest, child_size).await?);
            }
            blobs.push(manifest);
            retained.push(descriptor.clone());
        }
        Ok((retained, blobs))
    }

    async fn push_referrer_material(
        &self,
        layout: &Path,
        reference: &RegistryReference,
        subject_digest: &str,
    ) -> Result<u64, StoreError> {
        let index: Value = serde_json::from_slice(&read_bounded(
            &layout.join("index.json"),
            MAX_MANIFEST_BYTES,
        )?)
        .map_err(adapter)?;
        let descriptors = index
            .get("manifests")
            .and_then(Value::as_array)
            .ok_or_else(|| StoreError::Adapter("OCI index manifests are required".into()))?;
        let mut transferred = 0_u64;
        for descriptor in descriptors {
            let digest = descriptor
                .get("digest")
                .and_then(Value::as_str)
                .ok_or_else(|| StoreError::Adapter("OCI descriptor digest is absent".into()))?;
            if digest == subject_digest {
                continue;
            }
            let size = descriptor
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| StoreError::Adapter("OCI descriptor size is absent".into()))?;
            if size > MAX_MANIFEST_BYTES {
                return Err(StoreError::Adapter(
                    "OCI referrer manifest exceeds 1 MiB".into(),
                ));
            }
            let manifest = read_layout_blob(layout, digest, size, MAX_MANIFEST_BYTES)?;
            let value: Value = serde_json::from_slice(&manifest).map_err(adapter)?;
            if value
                .get("subject")
                .and_then(|subject| subject.get("digest"))
                .and_then(Value::as_str)
                != Some(subject_digest)
            {
                continue;
            }
            let mut children = Vec::new();
            if let Some(config) = value.get("config") {
                children.push(config);
            }
            if let Some(layers) = value.get("layers").and_then(Value::as_array) {
                children.extend(layers);
            }
            for child in children {
                let child_digest = child
                    .get("digest")
                    .and_then(Value::as_str)
                    .ok_or_else(|| StoreError::Adapter("referrer blob digest is absent".into()))?;
                let child_size = child
                    .get("size")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| StoreError::Adapter("referrer blob size is absent".into()))?;
                if child_size > MAX_FILE_BYTES {
                    return Err(StoreError::Adapter(
                        "OCI referrer blob exceeds 256 MiB".into(),
                    ));
                }
                let bytes = read_layout_blob(layout, child_digest, child_size, MAX_FILE_BYTES)?;
                self.push_blob(reference, child_digest, &bytes).await?;
                transferred = transferred
                    .checked_add(child_size)
                    .ok_or_else(|| StoreError::Adapter("registry byte count overflow".into()))?;
            }
            let url = self.registry_url(reference, "manifests", digest)?;
            let response = self
                .authenticated_send(
                    Method::PUT,
                    url,
                    Some(format!("repository:{}:push", reference.repository)),
                    Some(OCI_IMAGE_MANIFEST_MEDIA_TYPE),
                    manifest,
                )
                .await?;
            if !matches!(
                response.status(),
                StatusCode::CREATED | StatusCode::ACCEPTED
            ) {
                return Err(registry_status("referrer manifest push", response.status()));
            }
            transferred = transferred
                .checked_add(size)
                .ok_or_else(|| StoreError::Adapter("registry byte count overflow".into()))?;
        }
        Ok(transferred)
    }

    async fn authenticated_head(
        &self,
        url: Url,
        scope: Option<String>,
    ) -> Result<Response, StoreError> {
        self.authenticated_request(Method::HEAD, url, scope, None, Vec::new(), false)
            .await
    }

    async fn authenticated_get(
        &self,
        url: Url,
        scope: Option<String>,
        redirects: bool,
    ) -> Result<Response, StoreError> {
        let mut response = self
            .authenticated_request(Method::GET, url, scope.clone(), None, Vec::new(), false)
            .await?;
        if !redirects {
            return Ok(response);
        }
        for _ in 0..MAX_REGISTRY_REDIRECTS {
            if !response.status().is_redirection() {
                return Ok(response);
            }
            let previous = response.url().clone();
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| StoreError::Adapter("blob redirect has no valid Location".into()))?;
            let target = previous.join(location).map_err(adapter)?;
            let target_origin = canonical_url_origin(&target)?;
            let registry_origin = canonical_origin(&self.profile.origin)?;
            if target_origin != registry_origin
                && !self.profile.blob_redirect_origins.iter().any(|origin| {
                    canonical_origin(origin).is_ok_and(|origin| origin == target_origin)
                })
            {
                return Err(StoreError::Adapter(format!(
                    "blob redirect origin is not permitted: {target_origin}"
                )));
            }
            // Authorization is deliberately stripped on every redirected request. Object-store
            // URLs carry their own bounded capability and never receive registry credentials.
            response = self
                .raw_request(Method::GET, target, None, None, Vec::new())
                .await?;
        }
        Err(StoreError::Adapter("blob redirect limit exceeded".into()))
    }

    async fn authenticated_send(
        &self,
        method: Method,
        url: Url,
        scope: Option<String>,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> Result<Response, StoreError> {
        self.authenticated_request(method, url, scope, content_type, body, false)
            .await
    }

    async fn authenticated_request(
        &self,
        method: Method,
        url: Url,
        scope: Option<String>,
        content_type: Option<&str>,
        body: Vec<u8>,
        _redirected: bool,
    ) -> Result<Response, StoreError> {
        let authorization = credential_authorization(&self.credential)?;
        let response = self
            .raw_request(
                method.clone(),
                url.clone(),
                authorization.as_deref(),
                content_type,
                body.clone(),
            )
            .await?;
        if response.status() != StatusCode::UNAUTHORIZED
            || matches!(self.credential, RegistryCredential::Bearer(_))
        {
            return Ok(response);
        }
        let challenge = response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                StoreError::Adapter("registry denied access without a challenge".into())
            })?;
        let challenge = parse_bearer_challenge(challenge)?;
        let token = self
            .fetch_bearer_token(&challenge, scope.as_deref())
            .await?;
        self.raw_request(
            method,
            url,
            Some(&format!("Bearer {token}")),
            content_type,
            body,
        )
        .await
    }

    async fn fetch_bearer_token(
        &self,
        challenge: &BearerChallenge,
        requested_scope: Option<&str>,
    ) -> Result<String, StoreError> {
        let origin = canonical_url_origin(&challenge.realm)?;
        if !self
            .profile
            .token_origins
            .iter()
            .any(|allowed| canonical_origin(allowed).is_ok_and(|allowed| allowed == origin))
        {
            return Err(StoreError::Adapter(format!(
                "Bearer token origin is not permitted: {origin}"
            )));
        }
        let mut url = challenge.realm.clone();
        {
            let mut query = url.query_pairs_mut();
            if let Some(service) = challenge.service.as_deref() {
                query.append_pair("service", service);
            }
            if let Some(scope) = requested_scope.or(challenge.scope.as_deref()) {
                query.append_pair("scope", scope);
            }
        }
        let authorization = match &self.credential {
            RegistryCredential::Basic { .. } => credential_authorization(&self.credential)?,
            RegistryCredential::Anonymous | RegistryCredential::Bearer(_) => None,
        };
        let response = self
            .raw_request(Method::GET, url, authorization.as_deref(), None, Vec::new())
            .await?;
        require_status(&response, StatusCode::OK, "Bearer token request")?;
        let token: TokenResponse =
            serde_json::from_slice(&read_response(response, MAX_TOKEN_BYTES).await?)
                .map_err(adapter)?;
        if token.token.is_empty() || token.token.len() > 64 * 1024 || token.token.contains('\0') {
            return Err(StoreError::Adapter(
                "Bearer token service returned an invalid token".into(),
            ));
        }
        Ok(token.token)
    }

    async fn raw_request(
        &self,
        method: Method,
        url: Url,
        authorization: Option<&str>,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> Result<Response, StoreError> {
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(adapter(
                "registry request URLs cannot contain user information or fragments",
            ));
        }
        let origin = canonical_url_origin(&url)?;
        let client = if let Some(client) = self.clients.lock().await.get(&origin).cloned() {
            client
        } else {
            let roots = self.roots_for(&url)?;
            let client =
                pinned_reqwest_client(&url, &roots, self.timeout_ms, self.profile.allow_non_public)
                    .await
                    .map_err(adapter)?;
            self.clients.lock().await.insert(origin, client.clone());
            client
        };
        let mut request = client
            .request(method, url)
            .header(header::USER_AGENT, "colossus-agent-plugins/1")
            .header(
                header::ACCEPT,
                format!(
                    "{OCI_IMAGE_MANIFEST_MEDIA_TYPE}, application/vnd.oci.image.index.v1+json, application/json"
                ),
            );
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        if !body.is_empty() {
            request = request.body(body);
        }
        request
            .send()
            .await
            .map_err(|error| adapter(error.without_url()))
    }

    fn roots_for(&self, url: &Url) -> Result<AdditionalRootCertificates, StoreError> {
        let origin = canonical_url_origin(url)?;
        let path = if origin == canonical_origin(&self.profile.origin)? {
            self.profile.ca_bundle_path.as_ref()
        } else if let Some(path) = self.profile.token_ca_bundle_paths.get(&origin) {
            Some(path)
        } else {
            self.profile.blob_redirect_ca_bundle_paths.get(&origin)
        };
        path.map_or_else(
            || Ok(AdditionalRootCertificates::default()),
            |path| AdditionalRootCertificates::from_pem_bundle_path(path).map_err(adapter),
        )
    }
}

fn validate_registry_profile(profile: &PluginRegistryProfile) -> Result<(), StoreError> {
    let origin = canonical_origin(&profile.origin)?;
    if profile.trust_profile.is_empty() {
        return Err(StoreError::Adapter(
            "registry profile must select a trust profile".into(),
        ));
    }
    for configured in profile
        .token_origins
        .iter()
        .chain(&profile.blob_redirect_origins)
    {
        canonical_origin(configured)?;
    }
    for configured in profile
        .token_ca_bundle_paths
        .keys()
        .chain(profile.blob_redirect_ca_bundle_paths.keys())
    {
        if canonical_origin(configured)? != *configured {
            return Err(StoreError::Adapter(
                "per-origin CA maps require canonical exact origins".into(),
            ));
        }
    }
    if origin.starts_with("http://") && !profile.allow_non_public {
        return Err(StoreError::Adapter(
            "HTTP registries require explicit allowNonPublic".into(),
        ));
    }
    Ok(())
}

fn validate_registry_credential(credential: &RegistryCredential) -> Result<(), StoreError> {
    let valid = match credential {
        RegistryCredential::Anonymous => true,
        RegistryCredential::Bearer(token) => !token.is_empty() && token.len() <= 64 * 1024,
        RegistryCredential::Basic { username, password } => {
            !username.is_empty()
                && username.len() <= 1024
                && !password.is_empty()
                && password.len() <= 64 * 1024
        }
    };
    if valid {
        Ok(())
    } else {
        Err(StoreError::Adapter("invalid registry credential".into()))
    }
}

fn credential_authorization(credential: &RegistryCredential) -> Result<Option<String>, StoreError> {
    match credential {
        RegistryCredential::Anonymous => Ok(None),
        RegistryCredential::Bearer(token) => Ok(Some(format!("Bearer {token}"))),
        RegistryCredential::Basic { username, password } => Ok(Some(format!(
            "Basic {}",
            BASE64.encode(format!("{username}:{password}"))
        ))),
    }
}

fn canonical_origin(value: &str) -> Result<String, StoreError> {
    let url = Url::parse(value).map_err(adapter)?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (url.path() != "/" && !url.path().is_empty())
        || !matches!(url.scheme(), "https" | "http")
    {
        return Err(StoreError::Adapter(
            "registry origins must be exact HTTP(S) origins without credentials or paths".into(),
        ));
    }
    canonical_url_origin(&url)
}

fn canonical_url_origin(url: &Url) -> Result<String, StoreError> {
    let host = url
        .host_str()
        .ok_or_else(|| StoreError::Adapter("URL has no host".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| StoreError::Adapter("URL has no port for its scheme".into()))?;
    let default = matches!((url.scheme(), port), ("https", 443) | ("http", 80));
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_ascii_lowercase()
    };
    Ok(if default {
        format!("{}://{host}", url.scheme())
    } else {
        format!("{}://{host}:{port}", url.scheme())
    })
}

fn validate_repository(repository: &str) -> Result<(), StoreError> {
    if repository.is_empty()
        || repository.len() > 1024
        || repository.starts_with('/')
        || repository.ends_with('/')
        || repository.split('/').any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
        })
    {
        Err(StoreError::Adapter("invalid OCI repository name".into()))
    } else {
        Ok(())
    }
}

fn validate_selector(selector: &str) -> Result<(), StoreError> {
    if selector.starts_with("sha256:") {
        return validate_digest(selector);
    }
    if selector.is_empty()
        || selector.len() > 128
        || !selector.as_bytes()[0].is_ascii_alphanumeric()
        || !selector
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Err(StoreError::Adapter("invalid OCI tag".into()))
    } else {
        Ok(())
    }
}

fn validate_digest(digest: &str) -> Result<(), StoreError> {
    if digest
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "OCI digest must be sha256:<hex>".into(),
        ))
    }
}

fn parse_bearer_challenge(value: &str) -> Result<BearerChallenge, StoreError> {
    let parameters = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .ok_or_else(|| {
            StoreError::Adapter("unsupported registry authentication challenge".into())
        })?;
    let mut values = BTreeMap::new();
    for item in split_quoted(parameters)? {
        let (name, value) = item
            .split_once('=')
            .ok_or_else(|| StoreError::Adapter("invalid Bearer challenge parameter".into()))?;
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .ok_or_else(|| StoreError::Adapter("Bearer challenge values must be quoted".into()))?;
        if value.contains(['\\', '"', '\0']) {
            return Err(StoreError::Adapter(
                "Bearer challenge contains unsupported escaping".into(),
            ));
        }
        values.insert(name.trim().to_ascii_lowercase(), value.to_owned());
    }
    let realm = values
        .remove("realm")
        .ok_or_else(|| StoreError::Adapter("Bearer challenge has no realm".into()))?;
    Ok(BearerChallenge {
        realm: Url::parse(&realm).map_err(adapter)?,
        service: values.remove("service"),
        scope: values.remove("scope"),
    })
}

fn split_quoted(value: &str) -> Result<Vec<&str>, StoreError> {
    let mut quoted = false;
    let mut start = 0;
    let mut output = Vec::new();
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                output.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    if quoted {
        return Err(StoreError::Adapter(
            "Bearer challenge has an unclosed quote".into(),
        ));
    }
    output.push(value[start..].trim());
    Ok(output)
}

fn require_status(
    response: &Response,
    expected: StatusCode,
    operation: &str,
) -> Result<(), StoreError> {
    if response.status() == expected {
        Ok(())
    } else {
        Err(registry_status(operation, response.status()))
    }
}

fn require_media_type(response: &Response, expected: &str, label: &str) -> Result<(), StoreError> {
    let actual = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if actual == Some(expected) {
        Ok(())
    } else {
        Err(StoreError::Adapter(format!(
            "registry {label} has an unsupported media type"
        )))
    }
}

fn registry_status(operation: &str, status: StatusCode) -> StoreError {
    let detail = match status {
        StatusCode::UNAUTHORIZED => "authentication required",
        StatusCode::FORBIDDEN => "registry RBAC denied the operation",
        StatusCode::NOT_FOUND => "artifact was not found",
        StatusCode::TOO_MANY_REQUESTS => "registry rate limit exceeded",
        _ if status.is_server_error() => "registry is unavailable",
        _ => "registry returned an unexpected status",
    };
    StoreError::Adapter(format!("{operation} failed: {detail} ({status})"))
}

async fn read_response(response: Response, maximum: u64) -> Result<Vec<u8>, StoreError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum)
    {
        return Err(StoreError::Adapter(
            "registry response exceeds the configured bound".into(),
        ));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(adapter)?;
        if u64::try_from(bytes.len().saturating_add(chunk.len())).map_err(adapter)? > maximum {
            return Err(StoreError::Adapter(
                "registry response exceeds the configured bound".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn upload_location(base: &Url, response: &Response) -> Result<Url, StoreError> {
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| StoreError::Adapter("registry upload response has no Location".into()))?;
    let url = base.join(location).map_err(adapter)?;
    if canonical_url_origin(&url)? != canonical_url_origin(base)? {
        return Err(StoreError::Adapter(
            "registry upload Location changed origin".into(),
        ));
    }
    Ok(url)
}

#[cfg(test)]
#[path = "registry_acceptance.rs"]
mod acceptance;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_references_pin_digests_and_match_the_selected_origin() {
        let tagged = RegistryReference::parse("registry.example.test/team/plugin:v1")
            .expect("tagged reference");
        assert_eq!(tagged.origin, "https://registry.example.test");
        assert_eq!(tagged.repository, "team/plugin");
        assert_eq!(tagged.selector, "v1");
        assert!(!tagged.digest_pinned);

        let digest = format!("sha256:{}", "a".repeat(64));
        let pinned =
            RegistryReference::parse(&format!("registry.example.test/team/plugin@{digest}"))
                .expect("pinned reference");
        assert_eq!(pinned.selector, digest);
        assert!(pinned.digest_pinned);

        let client = PluginRegistryClient::new(
            PluginRegistryProfile {
                origin: "https://registry.example.test".into(),
                trust_profile: "required".into(),
                ..PluginRegistryProfile::default()
            },
            RegistryCredential::Anonymous,
        )
        .expect("registry client");
        assert!(
            client
                .reference("other.example.test/team/plugin:v1")
                .is_err()
        );
        assert!(RegistryReference::parse("registry.example.test/team/plugin").is_err());
    }

    #[test]
    fn bearer_challenges_are_strict_and_rbac_errors_are_actionable() {
        let challenge = parse_bearer_challenge(
            r#"Bearer realm="https://auth.example.test/token",service="registry.example.test",scope="repository:team/plugin:pull""#,
        )
        .expect("Bearer challenge");
        assert_eq!(challenge.realm.as_str(), "https://auth.example.test/token");
        assert_eq!(challenge.service.as_deref(), Some("registry.example.test"));
        assert_eq!(
            challenge.scope.as_deref(),
            Some("repository:team/plugin:pull")
        );
        assert!(parse_bearer_challenge("Basic realm=registry").is_err());
        assert!(parse_bearer_challenge(r#"Bearer service="registry""#).is_err());
        assert!(parse_bearer_challenge(r#"Bearer realm="https://auth.example/\\""#).is_err());
        assert!(
            registry_status("manifest push", StatusCode::FORBIDDEN)
                .to_string()
                .contains("RBAC denied")
        );
    }

    #[test]
    fn registry_profiles_require_canonical_separate_origin_policy() {
        let profile = PluginRegistryProfile {
            origin: "http://127.0.0.1:5000".into(),
            trust_profile: "required".into(),
            ..PluginRegistryProfile::default()
        };
        assert!(validate_registry_profile(&profile).is_err());

        let mut permitted = profile;
        permitted.allow_non_public = true;
        permitted.token_origins = vec!["https://auth.example.test".into()];
        permitted.blob_redirect_origins = vec!["https://objects.example.test".into()];
        assert!(validate_registry_profile(&permitted).is_ok());
        permitted.token_ca_bundle_paths.insert(
            "https://auth.example.test/path".into(),
            PathBuf::from("/tmp/ca.pem"),
        );
        assert!(validate_registry_profile(&permitted).is_err());
    }
}
