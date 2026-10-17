//! Signed fixed local synthetic inputs; no secret or signer fields in IPC.
use colossus_darwin_process::DarwinAuditPeer;
use colossus_native_browser_mac_identity::{
    Enrollment, NativeIdentity, NativeMutationAuthority, Scheme, Status,
};
use colossus_native_browser_tls_broker::{
    BrokerError, ClientIdentity, IdentityScheme, LocalHandshake, LocalIdentity,
};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    fs::OpenOptions,
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
    sync::Arc,
};
use zeroize::Zeroizing;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inputs {
    schema_version: u32,
    purpose: String,
    synthetic_fixture_only: bool,
    origin: String,
    identity_id: String,
    reviewed_leaf_sha256: String,
    ca_sha256: String,
    pkcs12_sha256: String,
    password_sha256: String,
}
pub(super) struct Prepared {
    owner: Arc<NativeIdentity>,
    origin: String,
    leaf: [u8; 32],
    metadata: [u8; 32],
}
#[derive(Debug)]
pub(super) enum Failure {
    Rejected,
    ImportUnknown,
    RetirementUnknown,
}
impl From<Status> for Failure {
    fn from(status: Status) -> Self {
        if matches!(status, Status::OutcomeUnknown) {
            Self::ImportUnknown
        } else {
            Self::Rejected
        }
    }
}
impl Failure {
    pub fn category(&self) -> &'static str {
        match self {
            Self::Rejected => "fixture_admission_or_import_rejected",
            Self::ImportUnknown => "fixture_import_unknown",
            Self::RetirementUnknown => "fixture_import_retirement_unknown",
        }
    }
}
pub(super) fn prepare(parent: &DarwinAuditPeer, generation: [u8; 32]) -> Result<Prepared, Failure> {
    let authority = Arc::new(NativeMutationAuthority::admit_acceptance(
        parent, generation,
    )?);
    let directory = authority.canonical_parent().join("acceptance-inputs");
    let metadata = colossus_native_browser_mac_identity::keychain_metadata_fingerprint()?;
    let config: Inputs = serde_json::from_slice(&read(&directory.join("identity.json"), 16384)?)
        .map_err(|_| Status::Denied)?;
    if config.schema_version != 1
        || config.purpose != "colossus-native-custody-synthetic-inputs"
        || !config.synthetic_fixture_only
        || config.identity_id != "synthetic-reviewed-client-p256"
    {
        return Err(Status::Denied.into());
    }
    let ca = read(&directory.join("private-ca.der"), 8192)?;
    let leaf = read(&directory.join("reviewed-client.der"), 65536)?;
    let pkcs12 = Zeroizing::new(read(
        &directory.join("encrypted-client.p12"),
        4 * 1024 * 1024,
    )?);
    let mut password = Zeroizing::new(read(&directory.join("enrollment-password"), 128)?);
    for (bytes, expected) in [
        (ca.as_slice(), &config.ca_sha256),
        (leaf.as_slice(), &config.reviewed_leaf_sha256),
        (pkcs12.as_slice(), &config.pkcs12_sha256),
        (password.as_slice(), &config.password_sha256),
    ] {
        if hex::encode(Sha256::digest(bytes)) != *expected {
            return Err(Status::Denied.into());
        }
    }
    let roots = vec![ca];
    let reviewed: [u8; 32] = Sha256::digest(&leaf).into();
    let enrolled = NativeIdentity::prepare(
        Enrollment {
            parent_fd: authority.parent_fd(),
            canonical_parent: authority.canonical_parent(),
            origin: &config.origin,
            code_hash: authority.broker_code_hash(),
            generation,
            ca_der: &roots,
            reviewed_leaf: &leaf,
            scheme: Scheme::EcdsaP256Sha256,
            encrypted_pkcs12: &pkcs12,
            password: password.as_mut(),
        },
        authority.clone(),
    );
    let owner = match enrolled {
        Ok(owner) => Arc::new(owner),
        Err(failure) => {
            if let Some(owner) = failure.owner {
                diagnostic("prepare", &owner);
                if owner.finish().is_err() {
                    diagnostic("retire", &owner);
                    return Err(Failure::RetirementUnknown);
                }
            }
            return Err(failure.status.into());
        }
    };
    reconcile_import_metadata(
        colossus_native_browser_mac_identity::keychain_metadata_fingerprint(),
        metadata,
        || owner.finish(),
    )?;
    Ok(Prepared {
        owner,
        origin: config.origin,
        leaf: reviewed,
        metadata,
    })
}
fn diagnostic(operation: &'static str, owner: &NativeIdentity) {
    use std::io::Write as _;
    if let Some(value) = owner.diagnostic() {
        let _ = writeln!(
            std::io::stderr().lock(),
            "colossus_native_custody_trace: {{\"operation\":\"{operation}\",\"phase\":{},\"check\":{},\"os_status\":{}}}",
            value.phase,
            value.check,
            value.os_status
        );
    }
}
fn reconcile_import_metadata(
    observed: Result<[u8; 32], Status>,
    expected: [u8; 32],
    finish: impl FnOnce() -> Result<(), Status>,
) -> Result<(), Failure> {
    if matches!(observed, Ok(current) if current == expected) {
        return Ok(());
    }
    // Import has already created an owned store. A read-only metadata error is
    // post-effect uncertainty, with the same explicit cleanup obligation as a
    // changed fingerprint; it can never fall through ordinary rejection/drop.
    if finish().is_err() {
        Err(Failure::RetirementUnknown)
    } else {
        Err(Failure::ImportUnknown)
    }
}
impl Prepared {
    pub fn identity(&self, generation: [u8; 32]) -> Result<ClientIdentity, Status> {
        if colossus_native_browser_mac_identity::keychain_metadata_fingerprint()? != self.metadata {
            return Err(Status::OutcomeUnknown);
        }
        ClientIdentity::new(
            self.origin.clone(),
            generation,
            self.leaf,
            Arc::new(MetadataIdentity {
                owner: Arc::clone(&self.owner),
                metadata: self.metadata,
            }),
        )
        .map_err(|_| Status::Denied)
    }
    pub fn finish(&self) -> Result<(), Status> {
        if let Err(status) = self.owner.finish() {
            diagnostic("retire", &self.owner);
            return Err(status);
        }
        if colossus_native_browser_mac_identity::keychain_metadata_fingerprint()? != self.metadata {
            return Err(Status::OutcomeUnknown);
        }
        Ok(())
    }
    pub fn revoke(&self) {
        self.owner.revoke();
    }
}
struct MetadataIdentity {
    owner: Arc<NativeIdentity>,
    metadata: [u8; 32],
}
struct MetadataHandshake {
    handshake: colossus_native_browser_mac_identity::NativeHandshake,
    metadata: [u8; 32],
}
fn metadata_unchanged(expected: &[u8; 32]) -> Result<(), BrokerError> {
    if colossus_native_browser_mac_identity::keychain_metadata_fingerprint()
        .map_err(|_| BrokerError::Unavailable)?
        == *expected
    {
        Ok(())
    } else {
        Err(BrokerError::OutcomeUnknown)
    }
}
impl LocalIdentity for MetadataIdentity {
    fn public_leaf(&self) -> Result<(Vec<u8>, IdentityScheme), BrokerError> {
        let (leaf, scheme) = self.owner.public_leaf();
        Ok((
            leaf.to_vec(),
            match scheme {
                Scheme::EcdsaP256Sha256 => IdentityScheme::EcdsaP256Sha256,
                Scheme::RsaPssSha256 => IdentityScheme::RsaPssSha256,
            },
        ))
    }
    fn begin(
        &self,
        generation: &[u8; 32],
        origin: &str,
        leaf: &[u8; 32],
        deadline: std::time::Instant,
    ) -> Result<Box<dyn LocalHandshake>, BrokerError> {
        metadata_unchanged(&self.metadata)?;
        let handshake = self
            .owner
            .begin(generation, origin, leaf, deadline)
            .map_err(|_| BrokerError::Unavailable)?;
        Ok(Box::new(MetadataHandshake {
            handshake,
            metadata: self.metadata,
        }))
    }
    fn revoke(&self) {
        self.owner.revoke();
    }
    fn finish(&self) -> Result<(), BrokerError> {
        if self.owner.finish().is_err() {
            diagnostic("retire", &self.owner);
            return Err(BrokerError::OutcomeUnknown);
        }
        metadata_unchanged(&self.metadata)
    }
}
impl LocalHandshake for MetadataHandshake {
    fn certificate_verify(
        &mut self,
        scheme: IdentityScheme,
        transcript: &[u8; 32],
    ) -> Result<Vec<u8>, BrokerError> {
        metadata_unchanged(&self.metadata)?;
        let signature = self
            .handshake
            .certificate_verify(
                match scheme {
                    IdentityScheme::EcdsaP256Sha256 => Scheme::EcdsaP256Sha256,
                    IdentityScheme::RsaPssSha256 => Scheme::RsaPssSha256,
                },
                transcript,
            )
            .map_err(|_| BrokerError::Unavailable)?;
        metadata_unchanged(&self.metadata)?;
        Ok(signature)
    }
}
fn read(path: &Path, maximum: u64) -> Result<Vec<u8>, Status> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(0x0100 | 0x01000000)
        .open(path)
        .map_err(|_| Status::Denied)?;
    let before = file.metadata().map_err(|_| Status::Denied)?;
    if !before.is_file()
        || before.nlink() != 1
        || before.mode() & 0o7777 != 0o600
        || before.len() == 0
        || before.len() > maximum
    {
        return Err(Status::Denied);
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Status::Denied)?;
    let after = path.symlink_metadata().map_err(|_| Status::Denied)?;
    if after.file_type().is_symlink()
        || bytes.len() as u64 != before.len()
        || (
            before.dev(),
            before.ino(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
    {
        return Err(Status::Denied);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn metadata_error_and_change_both_reconcile_positive_import() {
        for observed in [Err(Status::Unavailable), Ok([2; 32])] {
            let retired = Cell::new(false);
            assert!(matches!(
                reconcile_import_metadata(observed, [1; 32], || {
                    retired.set(true);
                    Ok(())
                }),
                Err(Failure::ImportUnknown)
            ));
            assert!(retired.get());
        }
    }
    #[test]
    fn metadata_error_with_failed_retirement_retains_cleanup_uncertainty() {
        assert!(matches!(
            reconcile_import_metadata(Err(Status::Unavailable), [1; 32], || {
                Err(Status::OutcomeUnknown)
            }),
            Err(Failure::RetirementUnknown)
        ));
        assert!(
            reconcile_import_metadata(Ok([1; 32]), [1; 32], || {
                panic!("unchanged metadata must not retire a ready identity")
            })
            .is_ok()
        );
    }
}
