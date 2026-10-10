//! Trusted native PKI enrollment. Secrets and filesystem choices have no model/serde API.
use colossus_contracts::{BrowserOrigin, BrowserSessionBinding, HostSecret};
use colossus_home::ConfinedRoot;
use colossus_native_browser_pki::ca_der;
use colossus_ports::{BrowserDriverError, BrowserDriverOpenRequest};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};
use zeroize::Zeroizing;

/// Encrypted PKCS#12 and its independently held native secret. Never cloned or serialized.
pub struct OciBrowserIdentity {
    pkcs12: Zeroizing<Vec<u8>>,
    password: HostSecret,
}
impl OciBrowserIdentity {
    /// Take native secret ownership; reject unsupported pinned NSS password formats.
    pub fn new(
        pkcs12: Zeroizing<Vec<u8>>,
        password: HostSecret,
    ) -> Result<Self, BrowserDriverError> {
        if pkcs12.is_empty()
            || pkcs12.len() > 4 * 1024 * 1024
            || !valid_password(password.expose().as_bytes())
        {
            return Err(BrowserDriverError::Denied);
        }
        Ok(Self { pkcs12, password })
    }

    pub(super) fn duplicate(&self) -> Self {
        Self {
            pkcs12: Zeroizing::new(self.pkcs12.to_vec()),
            password: self.password.duplicate(),
        }
    }

    pub(super) fn encrypted_bytes(&self) -> usize {
        self.pkcs12.len()
    }
}

/// One trusted public identity policy; no origin/key selector is deserializable.
pub struct OciClientIdentityBinding {
    /// Exact HTTPS authority admitted by native enrollment.
    pub origin: BrowserOrigin,
    /// SHA-256 of the public leaf DER; the native offered certificate must match.
    pub fingerprint_sha256: String,
}

/// Material bound to one immutable runtime/workspace/application/conversation owner.
pub struct OciBrowserPki {
    owner: BrowserSessionBinding,
    cas: Vec<Vec<u8>>,
    identities: Vec<OciBrowserIdentity>,
    bindings: Vec<OciClientIdentityBinding>,
}
impl OciBrowserPki {
    /// Validate reviewed native material before retaining any launch authority.
    pub fn new(
        owner: BrowserSessionBinding,
        ca_certificates: Vec<Vec<u8>>,
        identities: Vec<OciBrowserIdentity>,
        bindings: Vec<OciClientIdentityBinding>,
    ) -> Result<Self, BrowserDriverError> {
        if ca_certificates.len() > 32 || identities.len() > 32 || bindings.len() > 32 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        if identities.is_empty() != bindings.is_empty() {
            return Err(BrowserDriverError::Denied);
        }
        let mut cas = Vec::with_capacity(ca_certificates.len());
        for certificate in ca_certificates {
            let der = ca_der(&certificate).map_err(|_| BrowserDriverError::Denied)?;
            if cas.contains(&der) {
                return Err(BrowserDriverError::Denied);
            }
            cas.push(der);
        }
        for (index, binding) in bindings.iter().enumerate() {
            if !binding.origin.as_str().starts_with("https://")
                || !valid_fingerprint(&binding.fingerprint_sha256)
                || bindings[..index]
                    .iter()
                    .any(|other| other.origin == binding.origin)
            {
                return Err(BrowserDriverError::Denied);
            }
        }
        Ok(Self {
            owner,
            cas,
            identities,
            bindings,
        })
    }

    /// Actual retained reviewed CA material, separate from installed acceptance.
    pub fn has_private_ca(&self) -> bool {
        !self.cas.is_empty()
    }
    /// Actual retained secret material and public origin bindings, separate from acceptance.
    pub fn has_client_identities(&self) -> bool {
        !self.identities.is_empty() && !self.bindings.is_empty()
    }

    /// Identity material that will actually be staged for this immutable origin envelope.
    pub fn has_client_identities_for(&self, origins: &[BrowserOrigin]) -> bool {
        !self.identities.is_empty()
            && self
                .bindings
                .iter()
                .any(|binding| origins.contains(&binding.origin))
    }

    /// Bind installation evidence to public ownership/trust policy, never password/key bytes.
    pub(super) fn policy_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"colossus-browser-oci-pki-policy-v1\0");
        // Typed public ownership has an infallible JSON representation; no secrets are present.
        digest
            .update(serde_json::to_vec(&self.owner).expect("public browser ownership serializes"));
        for ca in &self.cas {
            digest.update(Sha256::digest(ca));
        }
        for binding in &self.bindings {
            digest.update((binding.origin.as_str().len() as u64).to_be_bytes());
            digest.update(binding.origin.as_str().as_bytes());
            digest.update(binding.fingerprint_sha256.as_bytes());
        }
        digest.finalize().into()
    }

    pub(super) fn stage(
        &self,
        control: &ConfinedRoot,
        request: &BrowserDriverOpenRequest,
    ) -> Result<StagedPki, BrowserDriverError> {
        if request.binding != self.owner {
            return Err(BrowserDriverError::Denied);
        }
        // Trust may expire while the credential source is retained between hosts.
        for ca in &self.cas {
            ca_der(ca).map_err(|_| BrowserDriverError::Denied)?;
        }
        let path = control
            .prepare_directory(Path::new("pki"))
            .map_err(|_| BrowserDriverError::Denied)?;
        let root = ConfinedRoot::bind(&path).map_err(|_| BrowserDriverError::Denied)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::DIRECTORY).bits() as i32,
            )
            .open(&path)
            .map_err(|_| BrowserDriverError::Denied)?;
        let mut result = BootstrapPki {
            require_source_retirement: true,
            certutil: tool("certutil")?,
            pk12util: tool("pk12util")?,
            ca_files: Vec::new(),
            identities: Vec::new(),
            bindings: Vec::new(),
        };
        let mut inputs = Vec::new();
        for (index, ca) in self.cas.iter().enumerate() {
            let name = format!("ca-{index}.der");
            write(&root, &name, ca)?;
            result.ca_files.push(native_path(&name));
        }
        for binding in &self.bindings {
            if request.options.allowed_origins.contains(&binding.origin) {
                result.bindings.push(BootstrapBinding {
                    origin: binding.origin.as_str().to_owned(),
                    fingerprint_sha256: binding.fingerprint_sha256.clone(),
                });
            }
        }
        // A narrower envelope with no enrolled authority receives no identity material.
        if !result.bindings.is_empty() {
            for (index, identity) in self.identities.iter().enumerate() {
                let pfx = format!("identity-{index}.pfx");
                let password = format!("password-{index}");
                inputs.push(OwnedInput {
                    name: pfx.clone(),
                    file: write(&root, &pfx, &identity.pkcs12)?,
                    retired: false,
                });
                inputs.push(OwnedInput {
                    name: password.clone(),
                    file: write(&root, &password, identity.password.expose().as_bytes())?,
                    retired: false,
                });
                result.identities.push(BootstrapIdentity {
                    pfx_file: native_path(&pfx),
                    passphrase_file: native_path(&password),
                });
            }
        }
        Ok(StagedPki {
            bootstrap: result,
            directory,
            inputs,
        })
    }
}

struct OwnedInput {
    name: String,
    file: File,
    retired: bool,
}

/// Retain exactly the newly created inputs until native provisioning acknowledges.
/// Unknown replacement/quarantine ownership is never inferred from a filename.
pub(super) struct StagedPki {
    bootstrap: BootstrapPki,
    directory: File,
    inputs: Vec<OwnedInput>,
}
impl StagedPki {
    pub(super) fn bootstrap(&self) -> &BootstrapPki {
        &self.bootstrap
    }

    pub(super) fn release_inputs(&mut self) -> Result<(), BrowserDriverError> {
        for input in &mut self.inputs {
            if input.retired {
                continue;
            }
            let held = input
                .file
                .metadata()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if !held.is_file() || held.nlink() != 1 || held.mode() & 0o077 != 0 {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            let quarantine = format!(".retired-input-{}", super::engine::nonce()?);
            rustix::fs::renameat_with(
                &self.directory,
                &input.name,
                &self.directory,
                &quarantine,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            let moved = rustix::fs::statat(
                &self.directory,
                &quarantine,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if moved.st_dev != held.dev()
                || moved.st_ino != held.ino()
                || moved.st_nlink != 1
                || moved.st_uid != held.uid()
            {
                // Preserve the moved unknown entry. Caller must retain the
                // control root and withhold startup ACK until ownership resolves.
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            rustix::fs::unlinkat(&self.directory, &quarantine, rustix::fs::AtFlags::empty())
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            input.retired = true;
        }
        self.directory
            .sync_all()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        // Closing the retained references is part of the pre-CEF input barrier.
        self.inputs.clear();
        Ok(())
    }
}

#[derive(Serialize)]
pub(super) struct BootstrapPki {
    require_source_retirement: bool,
    certutil: BootstrapTool,
    pk12util: BootstrapTool,
    ca_files: Vec<String>,
    identities: Vec<BootstrapIdentity>,
    bindings: Vec<BootstrapBinding>,
}
#[derive(Serialize)]
struct BootstrapTool {
    path: String,
    sha256: String,
}
#[derive(Serialize)]
struct BootstrapIdentity {
    pfx_file: String,
    passphrase_file: String,
}
#[derive(Serialize)]
struct BootstrapBinding {
    origin: String,
    fingerprint_sha256: String,
}

fn native_path(name: &str) -> String {
    format!("/run/colossus-browser-control/pki/{name}")
}
fn valid_password(password: &[u8]) -> bool {
    !password.is_empty()
        && password.len() <= 128
        && password.iter().all(|byte| (32..=126).contains(byte))
}
pub(super) fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn write(root: &ConfinedRoot, name: &str, bytes: &[u8]) -> Result<File, BrowserDriverError> {
    let file = root
        .open_file(Path::new(name))
        .map_err(|_| BrowserDriverError::Denied)?;
    if !file.was_created() {
        return Err(BrowserDriverError::Denied);
    }
    file.file()
        .write_all(bytes)
        .and_then(|()| file.file().sync_all())
        .map_err(|_| BrowserDriverError::Unavailable)?;
    file.file()
        .try_clone()
        .map_err(|_| BrowserDriverError::Unavailable)
}

#[derive(Deserialize)]
struct Lock {
    artifacts: std::collections::BTreeMap<String, Artifact>,
}
#[derive(Deserialize)]
struct Artifact {
    sha256: String,
}
fn tool(name: &str) -> Result<BootstrapTool, BrowserDriverError> {
    let lock: Lock = serde_json::from_str(include_str!(
        "../../../../native/browser/nss-tools.lock.json"
    ))
    .map_err(|_| BrowserDriverError::Unavailable)?;
    let artifact = lock
        .artifacts
        .get(name)
        .ok_or(BrowserDriverError::Unavailable)?;
    Ok(BootstrapTool {
        path: format!("/usr/bin/{name}"),
        sha256: artifact.sha256.clone(),
    })
}

#[cfg(test)]
mod tests;
