//! Bounded native-composition setup; never accepted on the browser protocol.

use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
};

const MARKER: &[u8; 8] = b"CDEP0001";
const MAX_SETUP: usize = 64 * 1024;

/// One immutable role image from independently verified native package metadata.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentImage {
    /// Zero for Main; one through four for the fixed helper role.
    pub role: u8,
    /// Canonical no-exec front selected only by trusted native composition.
    pub executable: PathBuf,
    /// Exact signed front code seal.
    pub cdhash: [u8; 20],
    /// Exact sibling body file digest.
    pub body_sha256: [u8; 32],
}

/// Setup sent only after both native processes independently admit signed Core.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentSetup {
    /// Nonzero native allocation nonce.
    pub nonce: [u8; 16],
    /// Exact sealed role-policy resource, independently verified by native Core.
    pub policy_sha256: [u8; 32],
    /// Fixed Main and helper images from trusted package verification.
    pub images: Vec<DeploymentImage>,
    /// Reviewed complete native environment. No browser frame has this authority.
    pub environment: Vec<String>,
}

impl DeploymentSetup {
    /// Validate finite transport and the exact role set before process creation.
    pub fn validate(&self) -> io::Result<()> {
        if self.nonce == [0; 16]
            || self.policy_sha256 == [0; 32]
            || self.images.len() != 5
            || self.environment.len() != 8
        {
            return Err(invalid("incomplete native deployment"));
        }
        let mut roles = std::collections::BTreeSet::new();
        for image in &self.images {
            if image.role > 4
                || !roles.insert(image.role)
                || !image.executable.is_absolute()
                || image
                    .executable
                    .to_str()
                    .is_none_or(|value| value.len() > 4096 || value.contains('\0'))
                || image.cdhash == [0; 20]
                || image.body_sha256 == [0; 32]
            {
                return Err(invalid("invalid native image inventory"));
            }
        }
        #[cfg(target_os = "macos")]
        {
            let allocation = self.allocation()?;
            crate::ProcessBroker::validate_images(
                allocation.nonce,
                &allocation.helpers,
                &allocation.environment,
            )?;
        }
        Ok(())
    }

    /// Write one bounded setup before opaque authenticated browser traffic.
    pub fn write(&self, destination: &mut impl Write) -> io::Result<()> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        if bytes.is_empty() || bytes.len() > MAX_SETUP {
            return Err(invalid("native deployment exceeds setup bound"));
        }
        destination.write_all(MARKER)?;
        destination.write_all(&(bytes.len() as u32).to_be_bytes())?;
        destination.write_all(&bytes)?;
        destination.flush()
    }

    /// Read one setup from a previously admitted signed native parent only.
    pub fn read(source: &mut impl Read) -> io::Result<Self> {
        let mut header = [0; 12];
        source.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header[8..].try_into().expect("fixed length")) as usize;
        if &header[..8] != MARKER || !(1..=MAX_SETUP).contains(&length) {
            return Err(invalid("invalid native deployment header"));
        }
        let mut bytes = vec![0; length];
        source.read_exact(&mut bytes)?;
        let value: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        value.validate()?;
        Ok(value)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn allocation(&self) -> io::Result<crate::CreatorAllocation> {
        let main = self
            .images
            .iter()
            .find(|image| image.role == 0)
            .ok_or_else(|| invalid("missing fixed Main image"))?;
        let helpers = self
            .images
            .iter()
            .filter(|image| image.role != 0)
            .map(|image| {
                Ok(crate::HelperImage {
                    role: crate::HelperRole::decode(image.role)?,
                    executable: image.executable.clone(),
                    cdhash: image.cdhash,
                    body_sha256: image.body_sha256,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(crate::CreatorAllocation {
            nonce: self.nonce,
            policy_sha256: self.policy_sha256,
            main: crate::MainImage {
                executable: main.executable.clone(),
                cdhash: main.cdhash,
                body_sha256: main.body_sha256,
            },
            helpers,
            environment: self
                .environment
                .iter()
                .map(std::ffi::OsString::from)
                .collect(),
        })
    }
}

/// Finite receipt from the independent actual parent, never whole-domain proof.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessReceipt {
    /// Exact allocation nonce binding, independent of any browser response.
    pub nonce: [u8; 16],
    /// Actual-parent raw Main wait status.
    pub main_status: Option<i32>,
    /// Exact known helpers reaped by their real creator parent.
    pub helpers_reaped: u16,
    /// Creation attempts without a positively retained returned child.
    pub unknown_creations: u16,
    /// Keeper positively observed the known enrolled roster empty.
    pub keeper_empty: bool,
    /// Main/helper output drains completed within their independent finite bound.
    pub output_drained: bool,
    /// All opaque relay workers completed within the finite bound.
    pub channels_closed: bool,
}

impl ProcessReceipt {
    /// Emit one finite receipt on the independent creator's dedicated stdout.
    pub fn write(&self, destination: &mut impl Write) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        destination.write_all(b"CSTAT001")?;
        destination.write_all(&(bytes.len() as u32).to_be_bytes())?;
        destination.write_all(&bytes)?;
        destination.flush()
    }

    /// Read a single finite receipt after exact creator exit has been reaped.
    pub fn read(source: &mut impl Read) -> io::Result<Self> {
        let mut header = [0; 12];
        source.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header[8..].try_into().expect("fixed length")) as usize;
        if &header[..8] != b"CSTAT001" || !(1..=2048).contains(&length) {
            return Err(invalid("invalid independent creator receipt"));
        }
        let mut bytes = vec![0; length];
        source.read_exact(&mut bytes)?;
        let value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let mut extra = [0];
        if source.read(&mut extra)? != 0 {
            return Err(invalid("multiple creator receipts"));
        }
        Ok(value)
    }

    /// Exact known-roster evidence; deliberately excludes a whole-process claim.
    #[must_use]
    pub fn known_children_closed(&self, nonce: [u8; 16]) -> bool {
        self.nonce == nonce
            && self.main_status.is_some()
            && self.unknown_creations == 0
            && self.keeper_empty
            && self.output_drained
            && self.channels_closed
    }
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unbounded_and_unknown_setup_is_rejected_before_allocation() {
        let mut header = MARKER.to_vec();
        header.extend_from_slice(&((MAX_SETUP + 1) as u32).to_be_bytes());
        assert!(DeploymentSetup::read(&mut header.as_slice()).is_err());
        let bytes = br#"{"nonce":[0],"images":[],"environment":[],"pid":42}"#;
        let mut framed = MARKER.to_vec();
        framed.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        framed.extend_from_slice(bytes);
        assert!(DeploymentSetup::read(&mut framed.as_slice()).is_err());
    }
    #[test]
    fn missing_wait_unknown_spawn_or_unsettled_transport_cannot_clear_roster() {
        let receipt = ProcessReceipt {
            nonce: [1; 16],
            main_status: Some(0),
            helpers_reaped: 4,
            unknown_creations: 0,
            keeper_empty: true,
            output_drained: true,
            channels_closed: true,
        };
        assert!(receipt.known_children_closed([1; 16]));
        assert!(!receipt.known_children_closed([2; 16]));
        for other in [
            ProcessReceipt {
                main_status: None,
                ..receipt
            },
            ProcessReceipt {
                unknown_creations: 1,
                ..receipt
            },
            ProcessReceipt {
                keeper_empty: false,
                ..receipt
            },
            ProcessReceipt {
                output_drained: false,
                ..receipt
            },
            ProcessReceipt {
                channels_closed: false,
                ..receipt
            },
        ] {
            assert!(!other.known_children_closed([1; 16]));
        }
    }
}
