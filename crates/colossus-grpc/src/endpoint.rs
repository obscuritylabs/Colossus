//! Secure local discovery for the loopback public API endpoint.
//!
//! The descriptor intentionally contains only public connection metadata. Bearer
//! credentials and private key material have no representation in this schema.

use crate::tls_identity::validate_end_entity_certificate;
use rustls::pki_types::{CertificateDer, pem::PemObject as _};
use serde::{Deserialize, Serialize};
use std::{fmt, io, path::Path};
use thiserror::Error;
use url::{Host, Url};
use uuid::Uuid;

#[cfg(unix)]
use getrandom::fill;
#[cfg(unix)]
use std::{
    fs,
    io::{Read as _, Write as _},
    path::PathBuf,
};

/// Current on-disk endpoint descriptor schema.
pub const ENDPOINT_DESCRIPTOR_SCHEMA_VERSION: u32 = 1;

/// Exact public API contract advertised by this transport.
pub const PUBLIC_API_VERSION: &str = "colossus.api.v1alpha1";

const MAX_DESCRIPTOR_BYTES: usize = 16 * 1024;
const MAX_CERTIFICATE_PEM_BYTES: usize = 256 * 1024;
const MAX_ENDPOINT_BYTES: usize = 2 * 1024;
#[cfg(unix)]
const TEMP_FILE_ATTEMPTS: usize = 8;

/// Non-secret metadata used to discover and pin a local Colossus API server.
///
/// Fields are private so callers cannot accidentally add a credential-bearing
/// extension. Deserialization also rejects unknown fields.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointDescriptor {
    schema_version: u32,
    api_version: String,
    instance_id: Uuid,
    endpoint: String,
    pid: u32,
    certificate_sha256: String,
}

impl EndpointDescriptor {
    /// Construct and validate a descriptor for a running local server.
    pub fn new(
        instance_id: Uuid,
        endpoint: impl Into<String>,
        pid: u32,
        certificate_sha256: impl Into<String>,
    ) -> Result<Self, EndpointDescriptorError> {
        let descriptor = Self {
            schema_version: ENDPOINT_DESCRIPTOR_SCHEMA_VERSION,
            api_version: PUBLIC_API_VERSION.into(),
            instance_id,
            endpoint: endpoint.into(),
            pid,
            certificate_sha256: certificate_sha256.into(),
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Descriptor schema version.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Advertised public API version.
    pub fn api_version(&self) -> &str {
        &self.api_version
    }

    /// Stable Colossus instance identifier.
    pub const fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    /// Canonical HTTPS loopback endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Process identifier of the server that published the descriptor.
    pub const fn pid(&self) -> u32 {
        self.pid
    }

    /// Lowercase SHA-256 fingerprint of the leaf certificate DER.
    pub fn certificate_sha256(&self) -> &str {
        &self.certificate_sha256
    }

    fn validate(&self) -> Result<(), EndpointDescriptorError> {
        if self.schema_version != ENDPOINT_DESCRIPTOR_SCHEMA_VERSION {
            return Err(EndpointDescriptorError::InvalidDescriptor(
                "unsupported endpoint descriptor schema version",
            ));
        }
        if self.api_version != PUBLIC_API_VERSION {
            return Err(EndpointDescriptorError::InvalidDescriptor(
                "unsupported public API version",
            ));
        }
        if self.instance_id.is_nil() {
            return Err(EndpointDescriptorError::InvalidDescriptor(
                "instance identifier must not be nil",
            ));
        }
        if self.pid == 0 {
            return Err(EndpointDescriptorError::InvalidDescriptor(
                "server process identifier must be non-zero",
            ));
        }
        validate_certificate_fingerprint(&self.certificate_sha256)?;
        validate_loopback_endpoint(&self.endpoint)
    }
}

impl fmt::Debug for EndpointDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointDescriptor")
            .field("schema_version", &self.schema_version)
            .field("api_version", &self.api_version)
            .field("instance_id", &self.instance_id)
            .field("endpoint", &self.endpoint)
            .field("pid", &self.pid)
            .field("certificate_sha256", &self.certificate_sha256)
            .finish()
    }
}

/// Failure to validate or securely persist endpoint discovery metadata.
#[derive(Debug, Error)]
pub enum EndpointDescriptorError {
    /// The discovery path is not an absolute file path.
    #[error("endpoint discovery path is invalid")]
    InvalidPath,
    /// The descriptor metadata violates a bounded public invariant.
    #[error("endpoint descriptor is invalid: {0}")]
    InvalidDescriptor(&'static str),
    /// The endpoint is not an exact, canonical HTTPS loopback URL.
    #[error("endpoint descriptor does not contain a canonical HTTPS loopback endpoint")]
    InvalidEndpoint,
    /// JSON is malformed, oversized, or contains an unrecognized field.
    #[error("endpoint descriptor encoding is invalid")]
    InvalidEncoding,
    /// Certificate PEM is malformed, oversized, or contains non-certificate material.
    #[error("public endpoint certificate PEM is invalid")]
    InvalidCertificatePem,
    /// Native owner-only storage is unavailable and no secure adapter was supplied.
    #[error("secure endpoint discovery storage is unsupported on this platform")]
    UnsupportedPlatform,
    /// The filesystem or platform security adapter rejected the operation.
    #[error("secure endpoint discovery storage operation failed")]
    Storage(#[source] io::Error),
}

/// Platform boundary for endpoint-discovery ACL and atomicity enforcement.
///
/// Implementations must reject links and aliases, verify that only the current
/// operating-system user can read or replace discovery files, write by atomic
/// same-directory replacement, and bound reads to `maximum_bytes`. The native
/// implementation provides those properties on Unix. Windows callers must inject
/// an implementation backed by explicit user-only DACL and reparse-point checks.
pub trait EndpointDescriptorStorage: Send + Sync {
    /// Atomically replace `path` with one owner-only metadata descriptor.
    fn write_atomic_owner_only(
        &self,
        path: &Path,
        descriptor: &EndpointDescriptor,
    ) -> io::Result<()>;

    /// Atomically replace `path` with one validated owner-only public certificate.
    ///
    /// The default fails closed so platforms without explicit ACL and link
    /// validation cannot silently publish TLS material. Implementations must
    /// reject private-key material and anything other than one `CA=false`
    /// end-entity certificate even when called directly.
    fn write_certificate_atomic_owner_only(
        &self,
        _path: &Path,
        _certificate_pem: &[u8],
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure public certificate storage is unavailable",
        ))
    }

    /// Read an owner-only regular discovery file without following links.
    fn read_owner_only(&self, path: &Path, maximum_bytes: usize) -> io::Result<Vec<u8>>;
}

/// Native secure storage implementation.
///
/// This implementation intentionally fails with `Unsupported` on platforms where
/// the required ownership and link checks have not been implemented.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeEndpointDescriptorStorage;

impl EndpointDescriptorStorage for NativeEndpointDescriptorStorage {
    fn write_atomic_owner_only(
        &self,
        path: &Path,
        descriptor: &EndpointDescriptor,
    ) -> io::Result<()> {
        native::write_atomic_owner_only(path, descriptor)
    }

    fn write_certificate_atomic_owner_only(
        &self,
        path: &Path,
        certificate_pem: &[u8],
    ) -> io::Result<()> {
        native::write_certificate_atomic_owner_only(path, certificate_pem)
    }

    fn read_owner_only(&self, path: &Path, maximum_bytes: usize) -> io::Result<Vec<u8>> {
        native::read_owner_only(path, maximum_bytes)
    }
}

/// Atomically publish a descriptor using native owner-only storage.
pub fn write_endpoint_descriptor(
    path: &Path,
    descriptor: &EndpointDescriptor,
) -> Result<(), EndpointDescriptorError> {
    write_endpoint_descriptor_with(path, descriptor, &NativeEndpointDescriptorStorage)
}

/// Atomically publish a descriptor using an explicit platform security adapter.
pub fn write_endpoint_descriptor_with(
    path: &Path,
    descriptor: &EndpointDescriptor,
    storage: &dyn EndpointDescriptorStorage,
) -> Result<(), EndpointDescriptorError> {
    validate_descriptor_path(path)?;
    descriptor.validate()?;
    storage
        .write_atomic_owner_only(path, descriptor)
        .map_err(map_storage_error)
}

/// Read and validate a descriptor using native owner-only storage.
pub fn read_endpoint_descriptor(
    path: &Path,
) -> Result<EndpointDescriptor, EndpointDescriptorError> {
    read_endpoint_descriptor_with(path, &NativeEndpointDescriptorStorage)
}

/// Read and validate a descriptor using an explicit platform security adapter.
pub fn read_endpoint_descriptor_with(
    path: &Path,
    storage: &dyn EndpointDescriptorStorage,
) -> Result<EndpointDescriptor, EndpointDescriptorError> {
    validate_descriptor_path(path)?;
    let encoded = storage
        .read_owner_only(path, MAX_DESCRIPTOR_BYTES)
        .map_err(map_storage_error)?;
    if encoded.is_empty() || encoded.len() > MAX_DESCRIPTOR_BYTES {
        return Err(EndpointDescriptorError::InvalidEncoding);
    }
    let descriptor = serde_json::from_slice::<EndpointDescriptor>(&encoded)
        .map_err(|_| EndpointDescriptorError::InvalidEncoding)?;
    descriptor.validate()?;
    Ok(descriptor)
}

/// Atomically publish a bounded public certificate using native owner-only storage.
pub fn write_endpoint_certificate(
    path: &Path,
    certificate_pem: &[u8],
) -> Result<(), EndpointDescriptorError> {
    write_endpoint_certificate_with(path, certificate_pem, &NativeEndpointDescriptorStorage)
}

/// Atomically publish a bounded public certificate through an explicit security adapter.
pub fn write_endpoint_certificate_with(
    path: &Path,
    certificate_pem: &[u8],
    storage: &dyn EndpointDescriptorStorage,
) -> Result<(), EndpointDescriptorError> {
    validate_descriptor_path(path)?;
    validate_endpoint_certificate_pem(certificate_pem)?;
    storage
        .write_certificate_atomic_owner_only(path, certificate_pem)
        .map_err(map_storage_error)
}

/// Read and validate a bounded public certificate using native owner-only storage.
pub fn read_endpoint_certificate(path: &Path) -> Result<Vec<u8>, EndpointDescriptorError> {
    read_endpoint_certificate_with(path, &NativeEndpointDescriptorStorage)
}

/// Read and validate a bounded public certificate through an explicit security adapter.
pub fn read_endpoint_certificate_with(
    path: &Path,
    storage: &dyn EndpointDescriptorStorage,
) -> Result<Vec<u8>, EndpointDescriptorError> {
    validate_descriptor_path(path)?;
    let certificate_pem = storage
        .read_owner_only(path, MAX_CERTIFICATE_PEM_BYTES)
        .map_err(map_storage_error)?;
    validate_endpoint_certificate_pem(&certificate_pem)?;
    Ok(certificate_pem)
}

fn validate_descriptor_path(path: &Path) -> Result<(), EndpointDescriptorError> {
    if !path.is_absolute() || path.file_name().is_none() || path.parent().is_none() {
        return Err(EndpointDescriptorError::InvalidPath);
    }
    Ok(())
}

/// Validate the exact public-certificate format accepted by endpoint discovery.
///
/// Colossus pins one end-entity certificate rather than trusting a CA chain. The PEM
/// therefore contains exactly one parsed certificate with explicit
/// `BasicConstraints CA=false`.
pub fn validate_endpoint_certificate_pem(
    certificate_pem: &[u8],
) -> Result<(), EndpointDescriptorError> {
    if certificate_pem.is_empty()
        || certificate_pem.len() > MAX_CERTIFICATE_PEM_BYTES
        || !certificate_pem.is_ascii()
        || contains_bytes(certificate_pem, b"PRIVATE KEY")
    {
        return Err(EndpointDescriptorError::InvalidCertificatePem);
    }

    let text = std::str::from_utf8(certificate_pem)
        .map_err(|_| EndpointDescriptorError::InvalidCertificatePem)?;
    let mut in_certificate = false;
    let mut has_body = false;
    let mut block_count = 0_usize;
    for line in text.lines() {
        if !in_certificate {
            if line.trim().is_empty() {
                continue;
            }
            if line != "-----BEGIN CERTIFICATE-----" {
                return Err(EndpointDescriptorError::InvalidCertificatePem);
            }
            in_certificate = true;
            has_body = false;
            continue;
        }

        if line == "-----END CERTIFICATE-----" {
            if !has_body {
                return Err(EndpointDescriptorError::InvalidCertificatePem);
            }
            block_count = block_count.saturating_add(1);
            if block_count > 1 {
                return Err(EndpointDescriptorError::InvalidCertificatePem);
            }
            in_certificate = false;
        } else if line.is_empty()
            || line == "-----BEGIN CERTIFICATE-----"
            || !line.bytes().all(is_base64_byte)
        {
            return Err(EndpointDescriptorError::InvalidCertificatePem);
        } else {
            has_body = true;
        }
    }
    if in_certificate || block_count != 1 {
        return Err(EndpointDescriptorError::InvalidCertificatePem);
    }

    let certificates = CertificateDer::pem_slice_iter(certificate_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| EndpointDescriptorError::InvalidCertificatePem)?;
    let [certificate] = certificates.as_slice() else {
        return Err(EndpointDescriptorError::InvalidCertificatePem);
    };
    validate_end_entity_certificate(certificate)
        .map_err(|_| EndpointDescriptorError::InvalidCertificatePem)?;
    Ok(())
}

fn is_base64_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn validate_certificate_fingerprint(value: &str) -> Result<(), EndpointDescriptorError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(EndpointDescriptorError::InvalidDescriptor(
            "certificate fingerprint must be a lowercase SHA-256 digest",
        ));
    }
    Ok(())
}

fn validate_loopback_endpoint(endpoint: &str) -> Result<(), EndpointDescriptorError> {
    if endpoint.is_empty() || endpoint.len() > MAX_ENDPOINT_BYTES || !endpoint.is_ascii() {
        return Err(EndpointDescriptorError::InvalidEndpoint);
    }
    let remainder = endpoint
        .strip_prefix("https://")
        .ok_or(EndpointDescriptorError::InvalidEndpoint)?;
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let suffix = &remainder[authority_end..];
    if !suffix.is_empty() && suffix != "/" {
        return Err(EndpointDescriptorError::InvalidEndpoint);
    }
    let port_text = if let Some(port) = authority.strip_prefix("127.0.0.1:") {
        port
    } else if let Some(port) = authority.strip_prefix("[::1]:") {
        port
    } else {
        return Err(EndpointDescriptorError::InvalidEndpoint);
    };
    let port = port_text
        .parse::<u16>()
        .map_err(|_| EndpointDescriptorError::InvalidEndpoint)?;
    if port == 0 || port_text != port.to_string() {
        return Err(EndpointDescriptorError::InvalidEndpoint);
    }

    let parsed = Url::parse(endpoint).map_err(|_| EndpointDescriptorError::InvalidEndpoint)?;
    let loopback_host = matches!(parsed.host(), Some(Host::Ipv4(address)) if address.is_loopback())
        || matches!(parsed.host(), Some(Host::Ipv6(address)) if address.is_loopback());
    if parsed.scheme() != "https"
        || !loopback_host
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
        || parsed.port_or_known_default() != Some(port)
    {
        return Err(EndpointDescriptorError::InvalidEndpoint);
    }
    Ok(())
}

fn map_storage_error(error: io::Error) -> EndpointDescriptorError {
    if error.kind() == io::ErrorKind::Unsupported {
        EndpointDescriptorError::UnsupportedPlatform
    } else {
        EndpointDescriptorError::Storage(error)
    }
}

#[cfg(unix)]
mod native {
    use super::*;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

    const DIRECTORY_MODE: u32 = 0o700;
    const DESCRIPTOR_MODE: u32 = 0o600;

    pub(super) fn write_atomic_owner_only(
        path: &Path,
        descriptor: &EndpointDescriptor,
    ) -> io::Result<()> {
        let mut contents = serde_json::to_vec(descriptor)
            .map_err(|_| invalid_data("descriptor encoding failed"))?;
        contents.push(b'\n');
        if contents.len() > MAX_DESCRIPTOR_BYTES {
            return Err(invalid_data("descriptor exceeds the maximum size"));
        }
        write_bytes_atomic_owner_only(path, &contents)
    }

    pub(super) fn write_certificate_atomic_owner_only(
        path: &Path,
        certificate_pem: &[u8],
    ) -> io::Result<()> {
        validate_endpoint_certificate_pem(certificate_pem)
            .map_err(|_| invalid_data("public certificate PEM is invalid"))?;
        write_bytes_atomic_owner_only(path, certificate_pem)
    }

    fn write_bytes_atomic_owner_only(path: &Path, contents: &[u8]) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| invalid_data("discovery file has no parent directory"))?;
        let parent_metadata = validate_directory(parent)?;
        validate_existing_destination(path, &parent_metadata)?;
        let temporary_path = create_temporary_path(parent)?;
        let result = (|| {
            let mut temporary = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(DESCRIPTOR_MODE)
                .open(&temporary_path)?;
            fs::set_permissions(&temporary_path, fs::Permissions::from_mode(DESCRIPTOR_MODE))?;
            temporary.write_all(contents)?;
            temporary.sync_all()?;
            let temporary_metadata = temporary.metadata()?;
            validate_regular_file(&temporary_metadata, parent_metadata.uid())?;
            drop(temporary);

            validate_existing_destination(path, &parent_metadata)?;
            fs::rename(&temporary_path, path)?;
            let published = fs::symlink_metadata(path)?;
            validate_regular_file(&published, parent_metadata.uid())?;
            fs::File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }

    pub(super) fn read_owner_only(path: &Path, maximum_bytes: usize) -> io::Result<Vec<u8>> {
        let parent = path
            .parent()
            .ok_or_else(|| invalid_data("discovery file has no parent directory"))?;
        let parent_metadata = validate_directory(parent)?;
        let before_open = fs::symlink_metadata(path)?;
        validate_regular_file(&before_open, parent_metadata.uid())?;

        let file = fs::OpenOptions::new().read(true).open(path)?;
        let after_open = file.metadata()?;
        validate_regular_file(&after_open, parent_metadata.uid())?;
        if before_open.dev() != after_open.dev() || before_open.ino() != after_open.ino() {
            return Err(invalid_data("discovery file changed while it was opened"));
        }
        if after_open.len() > maximum_bytes as u64 {
            return Err(invalid_data("discovery file exceeds the maximum size"));
        }

        let mut encoded = Vec::with_capacity(after_open.len() as usize);
        file.take(maximum_bytes.saturating_add(1) as u64)
            .read_to_end(&mut encoded)?;
        if encoded.len() > maximum_bytes {
            return Err(invalid_data("discovery file exceeds the maximum size"));
        }
        Ok(encoded)
    }

    fn validate_directory(path: &Path) -> io::Result<fs::Metadata> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.mode() & 0o777 != DIRECTORY_MODE
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(invalid_data(
                "discovery directory must be a current-user owner-only directory",
            ));
        }
        Ok(metadata)
    }

    fn validate_existing_destination(path: &Path, parent: &fs::Metadata) -> io::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => validate_regular_file(&metadata, parent.uid()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn validate_regular_file(metadata: &fs::Metadata, owner: u32) -> io::Result<()> {
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.mode() & 0o777 != DESCRIPTOR_MODE
            || metadata.uid() != owner
            || metadata.nlink() != 1
        {
            return Err(invalid_data(
                "discovery file must be a single-link owner-only regular file",
            ));
        }
        Ok(())
    }

    fn create_temporary_path(parent: &Path) -> io::Result<PathBuf> {
        for _ in 0..TEMP_FILE_ATTEMPTS {
            let mut random = [0_u8; 16];
            fill(&mut random)
                .map_err(|_| io::Error::other("secure temporary filename generation failed"))?;
            let name = format!(".colossus-endpoint-{}.tmp", lowercase_hex(&random));
            let path = parent.join(name);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(path),
                Ok(_) => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate an endpoint descriptor temporary file",
        ))
    }

    fn invalid_data(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use colossus_windows_native::{BoundPath, create_private_file, replace_private_file};
    use std::io::Read as _;

    pub(super) fn write_atomic_owner_only(
        path: &Path,
        descriptor: &EndpointDescriptor,
    ) -> io::Result<()> {
        let mut contents = serde_json::to_vec(descriptor)
            .map_err(|_| invalid_data("descriptor encoding failed"))?;
        contents.push(b'\n');
        if contents.len() > MAX_DESCRIPTOR_BYTES {
            return Err(invalid_data("descriptor exceeds the maximum size"));
        }
        write_bytes_atomic_owner_only(path, &contents)
    }

    pub(super) fn write_certificate_atomic_owner_only(
        path: &Path,
        certificate_pem: &[u8],
    ) -> io::Result<()> {
        validate_endpoint_certificate_pem(certificate_pem)
            .map_err(|_| invalid_data("public certificate PEM is invalid"))?;
        write_bytes_atomic_owner_only(path, certificate_pem)
    }

    fn write_bytes_atomic_owner_only(path: &Path, contents: &[u8]) -> io::Result<()> {
        let parent_path = path
            .parent()
            .ok_or_else(|| invalid_data("discovery file has no parent directory"))?;
        let parent = BoundPath::open_directory(parent_path).map_err(native_error)?;
        parent
            .validate_ancestor_namespace_authority()
            .and_then(|()| parent.validate_private_owner_dacl())
            .and_then(|()| parent.revalidate())
            .map_err(native_error)?;
        validate_existing_destination(path)?;

        let temporary_path = parent_path.join(format!(".colossus-endpoint-{}.tmp", Uuid::now_v7()));
        create_private_file(&temporary_path, contents).map_err(native_error)?;
        let result = (|| {
            parent.revalidate().map_err(native_error)?;
            validate_existing_destination(path)?;
            replace_private_file(&temporary_path, path).map_err(native_error)?;
            validate_file(path)?;
            parent.revalidate().map_err(native_error)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary_path);
        }
        result
    }

    pub(super) fn read_owner_only(path: &Path, maximum_bytes: usize) -> io::Result<Vec<u8>> {
        let binding = validate_file(path)?;
        let mut file = binding.try_clone_file().map_err(native_error)?;
        let length = file.metadata()?.len();
        if length > maximum_bytes as u64 {
            return Err(invalid_data("discovery file exceeds the maximum size"));
        }
        let mut encoded = Vec::with_capacity(length as usize);
        file.by_ref()
            .take(maximum_bytes.saturating_add(1) as u64)
            .read_to_end(&mut encoded)?;
        binding.revalidate().map_err(native_error)?;
        if encoded.len() > maximum_bytes {
            return Err(invalid_data("discovery file exceeds the maximum size"));
        }
        Ok(encoded)
    }

    fn validate_existing_destination(path: &Path) -> io::Result<()> {
        match validate_file(path) {
            Ok(_) => Ok(()),
            Err(validation_error) => match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                _ => Err(validation_error),
            },
        }
    }

    fn validate_file(path: &Path) -> io::Result<BoundPath> {
        let parent_path = path
            .parent()
            .ok_or_else(|| invalid_data("discovery file has no parent directory"))?;
        let parent = BoundPath::open_directory(parent_path).map_err(native_error)?;
        parent
            .validate_ancestor_namespace_authority()
            .and_then(|()| parent.validate_private_owner_dacl())
            .and_then(|()| parent.revalidate())
            .map_err(native_error)?;
        let binding = BoundPath::open_file(path).map_err(native_error)?;
        binding
            .validate_ancestor_namespace_authority()
            .and_then(|()| binding.validate_private_owner_dacl())
            .and_then(|()| binding.revalidate())
            .map_err(native_error)?;
        if binding.link_count().map_err(native_error)? != 1 {
            return Err(invalid_data(
                "discovery file must be a single-link owner-only regular file",
            ));
        }
        parent.revalidate().map_err(native_error)?;
        Ok(binding)
    }

    fn native_error(error: colossus_windows_native::WindowsNativeError) -> io::Error {
        io::Error::other(error)
    }

    fn invalid_data(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}

#[cfg(not(any(unix, windows)))]
mod native {
    use super::*;

    pub(super) fn write_atomic_owner_only(
        _path: &Path,
        _descriptor: &EndpointDescriptor,
    ) -> io::Result<()> {
        Err(unsupported())
    }

    pub(super) fn write_certificate_atomic_owner_only(
        _path: &Path,
        _certificate_pem: &[u8],
    ) -> io::Result<()> {
        Err(unsupported())
    }

    pub(super) fn read_owner_only(_path: &Path, _maximum_bytes: usize) -> io::Result<Vec<u8>> {
        Err(unsupported())
    }

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "native endpoint discovery ACL validation is unavailable",
        )
    }
}

#[cfg(unix)]
fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls_identity::{TlsIdentity, TlsKeySeed};

    fn descriptor(endpoint: &str) -> EndpointDescriptor {
        EndpointDescriptor::new(
            Uuid::parse_str("018f3f7a-36c6-7c8a-8e9f-c5e27ed955e8").expect("test UUID"),
            endpoint,
            42,
            "a".repeat(64),
        )
        .expect("valid descriptor")
    }

    fn certificate_pem() -> Vec<u8> {
        TlsIdentity::from_seed(TlsKeySeed::new([0x42_u8; 32]))
            .expect("test TLS identity")
            .certificate_pem()
            .to_vec()
    }

    struct DescriptorOnlyStorage;

    impl EndpointDescriptorStorage for DescriptorOnlyStorage {
        fn write_atomic_owner_only(
            &self,
            _path: &Path,
            _descriptor: &EndpointDescriptor,
        ) -> io::Result<()> {
            Ok(())
        }

        fn read_owner_only(&self, _path: &Path, _maximum_bytes: usize) -> io::Result<Vec<u8>> {
            Err(io::Error::new(io::ErrorKind::NotFound, "not used"))
        }
    }

    #[test]
    fn descriptor_has_no_credential_field_and_rejects_unknown_fields() {
        let descriptor = descriptor("https://127.0.0.1:4317");
        let mut value = serde_json::to_value(&descriptor).expect("descriptor JSON");
        let object = value.as_object_mut().expect("descriptor object");
        assert!(!object.keys().any(|key| {
            let key = key.to_ascii_lowercase();
            key.contains("token")
                || key.contains("secret")
                || key.contains("credential")
                || key.contains("authorization")
        }));
        object.insert("bearer_token".into(), serde_json::json!("must-not-parse"));
        assert!(serde_json::from_value::<EndpointDescriptor>(value).is_err());
    }

    #[test]
    fn exact_loopback_https_endpoints_are_accepted() {
        for endpoint in [
            "https://127.0.0.1:4317",
            "https://[::1]:4317",
            "https://127.0.0.1:4317/",
        ] {
            descriptor(endpoint);
        }
    }

    #[test]
    fn noncanonical_or_nonloopback_endpoints_are_rejected() {
        for endpoint in [
            "http://127.0.0.1:4317",
            "https://0.0.0.0:4317",
            "https://192.168.1.2:4317",
            "https://localhost:4317",
            "https://localhost.evil.invalid:4317",
            "https://LOCALHOST:4317",
            "https://2130706433:4317",
            "https://127.0.0.1",
            "https://127.0.0.1:0",
            "https://127.0.0.1:04317",
            "https://user@127.0.0.1:4317",
            "https://127.0.0.1:4317/rpc",
            "https://127.0.0.1:4317/?token=secret",
            "https://127.0.0.1:4317/#fragment",
        ] {
            assert!(
                EndpointDescriptor::new(
                    Uuid::parse_str("018f3f7a-36c6-7c8a-8e9f-c5e27ed955e8").expect("test UUID"),
                    endpoint,
                    42,
                    "a".repeat(64),
                )
                .is_err(),
                "unexpectedly accepted {endpoint}"
            );
        }
    }

    #[test]
    fn invalid_version_pid_and_fingerprint_are_rejected() {
        let valid = descriptor("https://127.0.0.1:4317");
        let mut version = valid.clone();
        version.api_version = "colossus.api.v2".into();
        assert!(version.validate().is_err());

        let mut pid = valid.clone();
        pid.pid = 0;
        assert!(pid.validate().is_err());

        for fingerprint in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
            let mut invalid = valid.clone();
            invalid.certificate_sha256 = fingerprint;
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn certificate_storage_adapter_defaults_to_fail_closed() {
        let path = std::env::current_dir()
            .expect("current directory")
            .join("endpoint-certificate-test.pem");
        assert!(matches!(
            write_endpoint_certificate_with(&path, &certificate_pem(), &DescriptorOnlyStorage),
            Err(EndpointDescriptorError::UnsupportedPlatform)
        ));
    }

    #[test]
    fn private_key_malformed_trailing_and_oversized_certificate_inputs_are_rejected() {
        let path = std::env::current_dir()
            .expect("current directory")
            .join("endpoint-certificate-invalid-test.pem");
        let private_key =
            b"-----BEGIN PRIVATE KEY-----\nYWJj\n-----END PRIVATE KEY-----\n".to_vec();
        let malformed_certificate =
            b"-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n".to_vec();
        let mut trailing_content = certificate_pem();
        trailing_content.extend_from_slice(b"Bearer must-not-be-published\n");
        let mut certificate_chain = certificate_pem();
        certificate_chain.extend_from_slice(&certificate_pem());
        let oversized = vec![b'A'; MAX_CERTIFICATE_PEM_BYTES + 1];

        for invalid in [
            private_key,
            malformed_certificate,
            trailing_content,
            certificate_chain,
            oversized,
        ] {
            assert!(matches!(
                write_endpoint_certificate_with(&path, &invalid, &DescriptorOnlyStorage),
                Err(EndpointDescriptorError::InvalidCertificatePem)
            ));
        }
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        struct TestDirectory(PathBuf);

        impl TestDirectory {
            fn new() -> Self {
                let mut random = [0_u8; 16];
                fill(&mut random).expect("test random");
                let path = std::env::temp_dir()
                    .join(format!("colossus-endpoint-test-{}", lowercase_hex(&random)));
                fs::create_dir(&path).expect("create test directory");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                    .expect("secure test directory");
                Self(path)
            }
        }

        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn atomic_round_trip_publishes_owner_only_regular_file() {
            let directory = TestDirectory::new();
            let path = directory.0.join("endpoint.json");
            let first = descriptor("https://127.0.0.1:4317");
            write_endpoint_descriptor(&path, &first).expect("first publish");
            assert_eq!(
                fs::symlink_metadata(&path)
                    .expect("descriptor metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(read_endpoint_descriptor(&path).expect("first read"), first);

            let second = descriptor("https://[::1]:4318");
            write_endpoint_descriptor(&path, &second).expect("atomic replacement");
            assert_eq!(
                read_endpoint_descriptor(&path).expect("replacement read"),
                second
            );
            assert!(
                fs::read_dir(&directory.0)
                    .expect("directory listing")
                    .all(|entry| !entry
                        .expect("directory entry")
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".tmp"))
            );
        }

        #[test]
        fn certificate_round_trip_uses_the_same_atomic_owner_only_boundary() {
            let directory = TestDirectory::new();
            let path = directory.0.join("server-certificate.pem");
            let certificate = certificate_pem();
            write_endpoint_certificate(&path, &certificate).expect("publish certificate");

            assert_eq!(
                fs::symlink_metadata(&path)
                    .expect("certificate metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                read_endpoint_certificate(&path).expect("read certificate"),
                certificate
            );
        }

        #[test]
        fn certificate_write_rejects_symlink_destination() {
            let directory = TestDirectory::new();
            let target = directory.0.join("target.pem");
            fs::write(&target, certificate_pem()).expect("target");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("target mode");
            let path = directory.0.join("server-certificate.pem");
            symlink(&target, &path).expect("certificate symlink");

            assert!(matches!(
                write_endpoint_certificate(&path, &certificate_pem()),
                Err(EndpointDescriptorError::Storage(_))
            ));
        }

        #[test]
        fn certificate_read_revalidates_file_contents() {
            let directory = TestDirectory::new();
            let path = directory.0.join("server-certificate.pem");
            fs::write(
                &path,
                b"-----BEGIN PRIVATE KEY-----\nYWJj\n-----END PRIVATE KEY-----\n",
            )
            .expect("invalid certificate file");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .expect("certificate mode");

            assert!(matches!(
                read_endpoint_certificate(&path),
                Err(EndpointDescriptorError::InvalidCertificatePem)
            ));
        }

        #[test]
        fn native_certificate_method_rejects_private_key_material_when_called_directly() {
            let directory = TestDirectory::new();
            let path = directory.0.join("server-certificate.pem");
            let storage = NativeEndpointDescriptorStorage;
            let error = storage
                .write_certificate_atomic_owner_only(
                    &path,
                    b"-----BEGIN PRIVATE KEY-----\nYWJj\n-----END PRIVATE KEY-----\n",
                )
                .expect_err("private key material must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(!path.exists());
        }

        #[test]
        fn symlink_descriptor_is_rejected_for_read_and_write() {
            let directory = TestDirectory::new();
            let target = directory.0.join("target.json");
            fs::write(&target, b"{}").expect("target");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("target mode");
            let path = directory.0.join("endpoint.json");
            symlink(&target, &path).expect("descriptor symlink");

            assert!(matches!(
                read_endpoint_descriptor(&path),
                Err(EndpointDescriptorError::Storage(_))
            ));
            assert!(matches!(
                write_endpoint_descriptor(&path, &descriptor("https://127.0.0.1:4317")),
                Err(EndpointDescriptorError::Storage(_))
            ));
        }

        #[test]
        fn symlink_parent_directory_is_rejected() {
            let directory = TestDirectory::new();
            let real_parent = directory.0.join("real");
            fs::create_dir(&real_parent).expect("real parent");
            fs::set_permissions(&real_parent, fs::Permissions::from_mode(0o700))
                .expect("real parent mode");
            let linked_parent = directory.0.join("linked");
            symlink(&real_parent, &linked_parent).expect("parent symlink");

            assert!(matches!(
                write_endpoint_descriptor(
                    &linked_parent.join("endpoint.json"),
                    &descriptor("https://127.0.0.1:4317")
                ),
                Err(EndpointDescriptorError::Storage(_))
            ));
        }

        #[test]
        fn hard_linked_descriptor_is_rejected() {
            let directory = TestDirectory::new();
            let path = directory.0.join("endpoint.json");
            write_endpoint_descriptor(&path, &descriptor("https://127.0.0.1:4317"))
                .expect("secure descriptor");
            fs::hard_link(&path, directory.0.join("alias.json")).expect("hard link");

            assert!(matches!(
                read_endpoint_descriptor(&path),
                Err(EndpointDescriptorError::Storage(_))
            ));
        }

        #[test]
        fn permissive_directory_or_descriptor_is_rejected() {
            let directory = TestDirectory::new();
            let path = directory.0.join("endpoint.json");
            fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755))
                .expect("permissive directory");
            assert!(matches!(
                write_endpoint_descriptor(&path, &descriptor("https://127.0.0.1:4317")),
                Err(EndpointDescriptorError::Storage(_))
            ));

            fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700))
                .expect("restore directory");
            write_endpoint_descriptor(&path, &descriptor("https://127.0.0.1:4317"))
                .expect("secure descriptor");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
                .expect("permissive descriptor");
            assert!(matches!(
                read_endpoint_descriptor(&path),
                Err(EndpointDescriptorError::Storage(_))
            ));
        }

        #[test]
        fn unknown_credential_field_is_rejected_from_secure_file() {
            let directory = TestDirectory::new();
            let path = directory.0.join("endpoint.json");
            let valid = descriptor("https://127.0.0.1:4317");
            let mut value = serde_json::to_value(valid).expect("JSON");
            value
                .as_object_mut()
                .expect("object")
                .insert("authorization".into(), serde_json::json!("Bearer secret"));
            fs::write(&path, serde_json::to_vec(&value).expect("encoded")).expect("descriptor");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("descriptor mode");

            assert!(matches!(
                read_endpoint_descriptor(&path),
                Err(EndpointDescriptorError::InvalidEncoding)
            ));
        }
    }
}
