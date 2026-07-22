use crate::{SdkError, SdkResult};
use std::{
    fmt,
    num::NonZeroU16,
    path::{Component, Path, PathBuf},
    str::FromStr,
};
use uuid::Uuid;

/// Stable UUID identifying one isolated Colossus instance.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InstanceId(Uuid);

impl InstanceId {
    /// Construct an instance identifier from a UUID.
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    /// Return the UUID value.
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    pub(crate) fn validate(self) -> SdkResult<()> {
        if self.0.is_nil() {
            return Err(SdkError::InvalidConfiguration(
                "instance identifier must not be nil",
            ));
        }
        Ok(())
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for InstanceId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Non-zero public API major version.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ApiMajor(NonZeroU16);

impl ApiMajor {
    /// Construct a non-zero major version.
    pub fn new(value: u16) -> SdkResult<Self> {
        NonZeroU16::new(value)
            .map(Self)
            .ok_or(SdkError::InvalidConfiguration(
                "API major version must be non-zero",
            ))
    }

    /// Return the numeric major version.
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

/// SHA-256 digest used to pin an installed executable.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Construct a digest from exact bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parse one lowercase or uppercase hexadecimal digest.
    pub fn from_hex(value: &str) -> SdkResult<Self> {
        let bytes = hex::decode(value)
            .map_err(|_| SdkError::InvalidConfiguration("invalid SHA-256 digest"))?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| SdkError::InvalidConfiguration("invalid SHA-256 digest length"))?;
        Ok(Self(bytes))
    }

    /// Borrow the exact digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Sha256Digest")
            .field(&hex::encode(self.0))
            .finish()
    }
}

/// SHA-256 fingerprint of the exact TLS leaf-certificate DER.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct TlsFingerprint(Sha256Digest);

impl TlsFingerprint {
    /// Construct a fingerprint from exact digest bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Sha256Digest::from_bytes(bytes))
    }

    /// Parse one canonical lowercase hexadecimal SHA-256 fingerprint.
    pub fn from_hex(value: &str) -> SdkResult<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(SdkError::InvalidConfiguration(
                "TLS fingerprint must be a lowercase SHA-256 digest",
            ));
        }
        Sha256Digest::from_hex(value).map(Self)
    }

    /// Borrow the fingerprint bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for TlsFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TlsFingerprint")
            .field(&hex::encode(self.as_bytes()))
            .finish()
    }
}

/// Absolute executable path paired with its signed-manifest SHA-256 identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedExecutable {
    path: PathBuf,
    sha256: Sha256Digest,
    macos_code_signing_requirement: MacosCodeSigningRequirement,
}

/// Expected macOS signing authority for a manifest-pinned bundled executable.
///
/// Stable/default callers require a matching canonical Apple TeamIdentifier. The ad-hoc
/// option exists only for an explicitly labeled Developer Preview whose trusted host has
/// already bound that release channel into its sealed bundle manifest.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MacosCodeSigningRequirement {
    /// Require the child and current executable to share one canonical Apple TeamIdentifier.
    #[default]
    AppleTeam,
    /// Require both the child and current executable to be ad-hoc signed without a TeamIdentifier.
    AdHocDeveloperPreview,
}

impl VerifiedExecutable {
    /// Create a pinned executable identity.
    ///
    /// The platform launcher must open the file without following an attacker-controlled
    /// leaf, verify this digest immediately before execution, and reject replacement
    /// races. This constructor only validates the portable shape.
    pub fn new(path: impl Into<PathBuf>, sha256: Sha256Digest) -> SdkResult<Self> {
        let path = absolute_non_root_path(path.into())?;
        Ok(Self {
            path,
            sha256,
            macos_code_signing_requirement: MacosCodeSigningRequirement::AppleTeam,
        })
    }

    /// Set the explicit macOS code-signing requirement selected by trusted host policy.
    #[must_use]
    pub const fn with_macos_code_signing_requirement(
        mut self,
        requirement: MacosCodeSigningRequirement,
    ) -> Self {
        self.macos_code_signing_requirement = requirement;
        self
    }

    /// Exact absolute path supplied by a signed installer or application bundle.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Required executable digest.
    pub const fn sha256(&self) -> Sha256Digest {
        self.sha256
    }

    /// Required macOS parent/child code-signing relationship.
    pub const fn macos_code_signing_requirement(&self) -> MacosCodeSigningRequirement {
        self.macos_code_signing_requirement
    }
}

/// Absolute application-private state directory for a sidecar or embedded runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPrivateInstanceDir(PathBuf);

impl AppPrivateInstanceDir {
    /// Validate an explicit application-private directory.
    ///
    /// Platform adapters must additionally reject symlinks, unsafe ownership, and
    /// group/world-writable ancestors before creating or opening canonical state.
    pub fn new(path: impl Into<PathBuf>) -> SdkResult<Self> {
        absolute_non_root_path(path.into()).map(Self)
    }

    /// Borrow the absolute directory.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

pub(crate) fn absolute_non_root_path(path: PathBuf) -> SdkResult<PathBuf> {
    if !path.is_absolute() {
        return Err(SdkError::PathNotAbsolute(path));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(SdkError::InvalidConfiguration(
            "application paths must not contain parent traversal",
        ));
    }
    if path.parent().is_none() || path.file_name().is_none() {
        return Err(SdkError::InvalidConfiguration(
            "filesystem root cannot be used as an application path",
        ));
    }
    Ok(path)
}
