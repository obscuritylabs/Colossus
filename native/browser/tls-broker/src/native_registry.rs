//! Actual private native registry, derived only from admitted package authority.
mod files;
mod identity;
mod model;
use crate::{
    BootstrapEnvelope, BrokerError, ClientIdentity, PkiSelection,
    registry_metadata::RegistryMetadata,
};
use colossus_native_browser_mac_identity::{
    Enrollment, MutationAuthority as _, NativeIdentity, NativeMutationAuthority, Scheme,
};
use files::{Directory, HeldFile};
use identity::{GuardedIdentity, PreparedIdentity};
use std::{
    fs::File,
    os::unix::fs::MetadataExt as _,
    sync::{Arc, Mutex},
};

/// Only dedicated native entry can construct this from a concrete independently
/// verified package/parent authority. No peer nominates trust, paths or signers.
pub struct NativeRegistry {
    authority: Arc<NativeMutationAuthority>,
    generation: [u8; 32],
    metadata: RegistryMetadata,
    metadata_file: HeldFile,
    directory: Directory,
    state: Mutex<State>,
}
struct State {
    attempted: bool,
    owner: Option<Arc<PreparedIdentity>>,
}
pub(crate) struct Resolved {
    pub envelope: BootstrapEnvelope,
    pub identity: Option<ClientIdentity>,
}
impl NativeRegistry {
    /// Compose only from a concrete admitted package/parent authority and that
    /// already authenticated generation; paths and public trust are fixed locally.
    pub fn admit(
        authority: NativeMutationAuthority,
        generation: [u8; 32],
    ) -> Result<Self, BrokerError> {
        let denied = BrokerError::InvalidBootstrap;
        if !authority.signing_admitted(&generation) {
            return Err(denied);
        }
        let expected = authority.registry_digest().ok_or(denied)?;
        let parent = File::from(
            authority
                .parent_fd()
                .try_clone_to_owned()
                .map_err(|_| denied)?,
        );
        let uid = parent.metadata().map_err(|_| denied)?.uid();
        let (metadata_file, bytes) = HeldFile::read(
            &authority.canonical_parent().join("owned-pki-registry.json"),
            128 * 1024,
            uid,
            &expected,
        )?;
        let metadata = RegistryMetadata::parse(&bytes).map_err(|_| denied)?;
        let directory = Directory::open(authority.canonical_parent().join("pki-enrollment"), uid)?;
        if !authority.signing_admitted(&generation) {
            return Err(denied);
        }
        // Only public metadata is opened here. CA/leaf and especially encrypted
        // container/password slots are read after the validated private command.
        Ok(Self {
            authority: Arc::new(authority),
            generation,
            metadata,
            metadata_file,
            directory,
            state: Mutex::new(State {
                attempted: false,
                owner: None,
            }),
        })
    }
    pub(crate) fn resolve(
        &self,
        mut envelope: BootstrapEnvelope,
        selection: PkiSelection,
    ) -> Result<Resolved, BrokerError> {
        let denied = BrokerError::InvalidBootstrap;
        if envelope.bootstrap.generation != self.generation
            || !self.authority.admits_tls(
                &self.generation,
                &envelope.bootstrap.origin,
                &envelope.bootstrap.destinations,
            )
        {
            return Err(denied);
        }
        self.metadata_file.revalidate()?;
        self.directory.revalidate()?;
        let plan = model::resolve(&self.metadata, &envelope.bootstrap.origin, &selection)?;
        let mut state = self.state.lock().map_err(|_| BrokerError::OutcomeUnknown)?;
        if state.attempted {
            return Err(denied);
        }
        state.attempted = true;
        let mut inputs = Vec::new();
        let mut roots = Vec::new();
        for index in plan.ca_indices {
            let ca = &self.metadata.cas[index];
            let (file, bytes) = HeldFile::read(
                &self.directory.path().join(format!("ca-{}.der", ca.slot)),
                1024 * 1024,
                self.directory.uid(),
                &files::digest(&ca.sha256)?,
            )?;
            let der = colossus_native_browser_pki::ca_der(&bytes).map_err(|_| denied)?;
            // Fixed inputs are DER; a PEM/DER hash ambiguity is rejected.
            if der.as_slice() != bytes.as_slice() {
                return Err(denied);
            }
            inputs.push(file);
            roots.push(der);
        }
        if roots.iter().map(Vec::len).sum::<usize>() > 1024 * 1024 {
            return Err(denied);
        }
        // Discard every received upstream root before TLS configuration. Only
        // exactly selected, signed native catalog material defines this trust.
        envelope.bootstrap.ca_der = roots;
        let Some(index) = plan.identity_index else {
            return Ok(Resolved {
                envelope,
                identity: None,
            });
        };
        let registered = &self.metadata.identities[index];
        let leaf_hash = files::digest(&registered.leaf_sha256)?;
        let (leaf_file, leaf) = HeldFile::read(
            &self
                .directory
                .path()
                .join(format!("client-{}.der", registered.slot)),
            65536,
            self.directory.uid(),
            &leaf_hash,
        )?;
        let (container_file, container) = HeldFile::read(
            &self
                .directory
                .path()
                .join(format!("client-{}.p12", registered.slot)),
            4 * 1024 * 1024,
            self.directory.uid(),
            &files::digest(&registered.pkcs12_sha256)?,
        )?;
        let (password_file, mut password) = HeldFile::read(
            &self
                .directory
                .path()
                .join(format!("password-{}", registered.slot)),
            128,
            self.directory.uid(),
            &files::digest(&registered.password_sha256)?,
        )?;
        inputs.extend([leaf_file, container_file, password_file]);
        self.directory.revalidate()?;
        self.metadata_file.revalidate()?;
        for input in &inputs {
            input.revalidate()?;
        }
        if !self.authority.signing_admitted(&self.generation) {
            return Err(denied);
        }
        let metadata = colossus_native_browser_mac_identity::keychain_metadata_fingerprint()
            .map_err(|_| BrokerError::Unavailable)?;
        let preparation = NativeIdentity::prepare(
            Enrollment {
                parent_fd: self.authority.parent_fd(),
                canonical_parent: self.authority.canonical_parent(),
                origin: &envelope.bootstrap.origin,
                code_hash: self.authority.broker_code_hash(),
                generation: self.generation,
                ca_der: &envelope.bootstrap.ca_der,
                reviewed_leaf: &leaf,
                // Current reviewed native acceptance covers this exact scheme.
                scheme: Scheme::EcdsaP256Sha256,
                encrypted_pkcs12: &container,
                password: password.as_mut(),
            },
            self.authority.clone(),
        );
        let (owner, failure) = match preparation {
            Ok(owner) => (owner, None),
            Err(failure) => match failure.owner {
                Some(owner) => (owner, Some(failure.status)),
                None => return Err(denied),
            },
        };
        let retained = Arc::new(PreparedIdentity {
            owner,
            inputs,
            metadata,
        });
        state.owner = Some(Arc::clone(&retained));
        // Retain the real partial owner before any fallible post-effect check.
        if failure.is_some() || retained.admitted().is_err() {
            return match retained.finish() {
                Ok(()) => Err(BrokerError::Unavailable),
                Err(_) => Err(BrokerError::OutcomeUnknown),
            };
        }
        let identity = ClientIdentity::new(
            envelope.bootstrap.origin.clone(),
            self.generation,
            leaf_hash,
            Arc::new(GuardedIdentity(retained)),
        )?;
        Ok(Resolved {
            envelope,
            identity: Some(identity),
        })
    }
    /// Immediately fence new signing on control/parent loss; never remove here.
    pub fn revoke(&self) {
        if let Ok(state) = self.state.lock()
            && let Some(owner) = &state.owner
        {
            owner.owner.revoke();
        }
    }
    /// Call only after positive tracked TLS drainage or before any task started.
    /// Unknown native removal/default metadata stays a retained durable duty.
    pub fn finish(&self) -> Result<(), BrokerError> {
        let state = self.state.lock().map_err(|_| BrokerError::OutcomeUnknown)?;
        match &state.owner {
            Some(owner) => owner.finish(),
            None => Ok(()),
        }
    }
}
