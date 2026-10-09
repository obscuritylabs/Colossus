use crate::ConnectionConfig;
use colossus_contracts::VaultRecord;
use colossus_ports::{CredentialKey, CredentialVault};
use rcgen::{CertificateParams, KeyPair};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};
mod renewal;
mod revocation;
pub(crate) use renewal::certificate_due;

/// Native enrollment store, shared by CLI and Desktop; secrets stay in the encrypted vault.
#[derive(Clone)]
pub struct EnrollmentStore {
    vault: Arc<dyn CredentialVault>,
    key: CredentialKey,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    config: Option<ConnectionConfig>,
    pending: Pending,
    #[serde(default)]
    rotation: Option<renewal::Rotation>,
}
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct Pending {
    client_key_pem: String,
    csr_pem: String,
    token: String,
    enrollment_url: String,
    instance_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enrolled {
    node: EnrolledNode,
    certificate_pem: String,
    ca_pem: String,
    grpc_endpoint: String,
}
// Only the enrollment binding crosses this native boundary. The cloud's
// relational repository and fleet presentation types are not runtime dependencies.
#[derive(Deserialize)]
struct EnrolledNode {
    project_id: String,
    node_id: String,
    instance_id: String,
}
impl EnrollmentStore {
    /// Exact native encrypted record selector for an explicitly reviewed named
    /// enrollment copy. This does not load a vault or create/rotate any grant.
    pub fn credential_key(id: &str) -> Result<CredentialKey, &'static str> {
        CredentialKey::new("cloud-connector", id).map_err(|_| "invalid enrollment identity")
    }
    /// Bind an opaque enrollment record under the caller's protected native vault.
    pub fn new(vault: Arc<dyn CredentialVault>, id: &str) -> Result<Self, &'static str> {
        Ok(Self {
            vault,
            key: Self::credential_key(id)?,
        })
    }
    /// Generate the key locally and persist the pending exchange before contacting
    /// the host. Retrying uses the same CSR and invitation after a lost response.
    pub async fn enroll(
        &self,
        url: String,
        token: Zeroizing<String>,
        instance_id: String,
        capabilities: Vec<String>,
        allow_loopback_http: bool,
    ) -> Result<ConnectionConfig, &'static str> {
        let endpoint = url::Url::parse(&url).map_err(|_| "invalid enrollment URL")?;
        let local = matches!(
            endpoint.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        );
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !(endpoint.scheme() == "https"
                || (allow_loopback_http && local && endpoint.scheme() == "http"))
        {
            return Err("enrollment requires HTTPS");
        }
        let store = self.clone();
        let existing = tokio::task::spawn_blocking(move || store.vault.read(&store.key))
            .await
            .map_err(|_| "enrollment vault unavailable")?
            .map_err(|_| "enrollment vault unavailable")?;
        let mut record: Record = if let Some(existing) = existing {
            serde_json::from_slice(existing.expose()).map_err(|_| "enrollment record invalid")?
        } else {
            if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("invalid enrollment invitation");
            }
            let key = KeyPair::generate().map_err(|_| "connector key generation failed")?;
            let csr = CertificateParams::default()
                .serialize_request(&key)
                .map_err(|_| "connector certificate request failed")?
                .pem()
                .map_err(|_| "connector certificate encoding failed")?;
            Record {
                config: None,
                rotation: None,
                pending: Pending {
                    client_key_pem: key.serialize_pem(),
                    csr_pem: csr,
                    token: token.to_string(),
                    enrollment_url: url.clone(),
                    instance_id: instance_id.clone(),
                },
            }
        };
        if record.pending.enrollment_url != url || record.pending.instance_id != instance_id {
            return Err("pending enrollment belongs to another runtime or cloud");
        }
        if let Some(config) = &record.config {
            return Ok(config.clone());
        }
        self.save_async(&record).await?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "enrollment HTTP unavailable")?;
        let response=client.post(&url).json(&serde_json::json!({"token":record.pending.token,"instance_id":record.pending.instance_id,"csr_pem":record.pending.csr_pem})).send().await.map_err(|_|"enrollment transport unavailable; retry the same exchange")?;
        if !response.status().is_success() {
            return Err("cloud enrollment rejected");
        }
        if response.content_length().is_some_and(|bytes| bytes > 65536) {
            return Err("enrollment response too large");
        }
        use futures::StreamExt as _;
        let mut body = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|_| "enrollment response unavailable")?;
            if bytes.len().saturating_add(chunk.len()) > 65536 {
                return Err("enrollment response too large");
            }
            bytes.extend_from_slice(&chunk);
        }
        let enrolled: Enrolled =
            serde_json::from_slice(&bytes).map_err(|_| "invalid enrollment response")?;
        if enrolled.node.instance_id != instance_id {
            return Err("enrolled runtime identity mismatch");
        }
        let config = ConnectionConfig {
            endpoint: enrolled.grpc_endpoint,
            project_id: enrolled.node.project_id,
            node_id: enrolled.node.node_id,
            instance_id: instance_id.clone(),
            certificate_pem: enrolled.certificate_pem,
            ca_pem: enrolled.ca_pem,
            capabilities,
            revoked: false,
            inventory: Some(crate::native_inventory(
                format!("workspace:{instance_id}"),
                "CLI workspace".into(),
                colossus_cloud_protocol::DeploymentKind::Cli,
            )?),
            shared_continuation: false,
        };
        record.config = Some(config.clone());
        record.pending.token.zeroize();
        self.save_async(&record).await?;
        Ok(config)
    }
    /// Load native enrollment and private TLS key without serializing secrets to UI.
    pub fn load(&self) -> Result<Option<(ConnectionConfig, Zeroizing<String>)>, &'static str> {
        let Some(record) = self
            .vault
            .read(&self.key)
            .map_err(|_| "enrollment vault unavailable")?
        else {
            return Ok(None);
        };
        let record: Record =
            serde_json::from_slice(record.expose()).map_err(|_| "enrollment record invalid")?;
        Ok(record.config.map(|config| {
            (
                config,
                Zeroizing::new(record.pending.client_key_pem.clone()),
            )
        }))
    }
    /// Remove the local enrollment after disconnect. This does not revoke the remote node.
    pub fn forget(&self) -> Result<(), &'static str> {
        self.vault
            .delete(&self.key)
            .map_err(|_| "enrollment cleanup failed")
    }
    /// Persist native-sourced presentation and sharing posture, without replacing authority.
    pub async fn set_inventory(
        &self,
        inventory: colossus_cloud_protocol::RuntimeInventory,
    ) -> Result<ConnectionConfig, &'static str> {
        inventory
            .validate()
            .map_err(|_| "invalid connector inventory")?;
        let store = self.clone();
        let record = tokio::task::spawn_blocking(move || store.vault.read(&store.key))
            .await
            .map_err(|_| "enrollment vault unavailable")?
            .map_err(|_| "enrollment vault unavailable")?
            .ok_or("enrollment unavailable")?;
        let mut record: Record =
            serde_json::from_slice(record.expose()).map_err(|_| "enrollment record invalid")?;
        let config = record.config.as_mut().ok_or("enrollment unavailable")?;
        config.inventory = Some(inventory);
        let config = config.clone();
        self.save_async(&record).await?;
        Ok(config)
    }
    /// Persist the confirmed native sharing posture after runtime authorization commits.
    pub async fn set_sharing(
        &self,
        enabled: bool,
        allow_continuation: bool,
    ) -> Result<ConnectionConfig, &'static str> {
        let store = self.clone();
        let record = tokio::task::spawn_blocking(move || store.vault.read(&store.key))
            .await
            .map_err(|_| "enrollment vault unavailable")?
            .map_err(|_| "enrollment vault unavailable")?
            .ok_or("enrollment unavailable")?;
        let mut record: Record =
            serde_json::from_slice(record.expose()).map_err(|_| "enrollment record invalid")?;
        let config = record.config.as_mut().ok_or("enrollment unavailable")?;
        let inventory = config
            .inventory
            .as_mut()
            .ok_or("native inventory unavailable")?;
        inventory.sharing = if enabled {
            crate::WorkspaceSharing::SharedVisibleSessions
        } else {
            crate::WorkspaceSharing::CloudOwned
        };
        config.shared_continuation = enabled && allow_continuation;
        let config = config.clone();
        self.save_async(&record).await?;
        Ok(config)
    }
    async fn save_async(&self, record: &Record) -> Result<(), &'static str> {
        let encoded =
            Zeroizing::new(serde_json::to_vec(record).map_err(|_| "enrollment encoding failed")?);
        let record =
            VaultRecord::new(encoded.to_vec()).map_err(|_| "enrollment record exceeds bound")?;
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.vault.write(&store.key, &record))
            .await
            .map_err(|_| "enrollment persistence failed")?
            .map_err(|_| "enrollment persistence failed")
    }
}
