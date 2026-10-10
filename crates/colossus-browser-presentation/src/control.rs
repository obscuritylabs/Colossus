//! Bounded native-only presentation control; independent of automation grants.
use colossus_contracts::{BrowserSessionId, BrowserTarget};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{Input, Lease, PresentationError};

/// Maximum authenticated control payload, excluding pixels.
pub const MAX_CONTROL_BYTES: usize = 16 * 1024;
/// Fixed header checked before allocating a control payload.
pub const CONTROL_HEADER_BYTES: usize = 84;
const MAGIC: &[u8; 8] = b"CLSPCTL1";

/// Separately authenticated direction of this private native channel.
#[derive(Clone, Copy)]
pub enum Role {
    /// Trusted native presenter requests to the dedicated browser host.
    NativeToHost,
    /// Dedicated browser host acknowledgments to the native presenter.
    HostToNative,
}

/// A native-owned opaque target and bounded logical viewport, never a HWND/NSView.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    /// Exact supervised opaque session.
    pub session: BrowserSessionId,
    /// Exact opaque tab and document observed by the trusted native owner.
    pub target: BrowserTarget,
    /// Current controller epoch; zero is separately admitted human ownership.
    pub control_generation: u64,
    /// Strictly increasing native viewport generation.
    pub viewport_generation: u64,
    /// Logical viewport width in device-independent pixels.
    pub width: u32,
    /// Logical viewport height in device-independent pixels.
    pub height: u32,
    /// Device scale multiplied by one thousand, bounded to 0.5 through 4.
    pub scale_milli: u32,
    /// Visibility heartbeat lease, at most 1500 milliseconds.
    pub lease_ms: u16,
}
impl Configure {
    /// Validate allocation bounds before native viewport effects.
    pub fn validate(&self) -> Result<(), PresentationError> {
        let width = (u64::from(self.width) * u64::from(self.scale_milli)).div_ceil(1000);
        let height = (u64::from(self.height) * u64::from(self.scale_milli)).div_ceil(1000);
        if self.viewport_generation == 0
            || !(500..=4000).contains(&self.scale_milli)
            || self.width == 0
            || self.height == 0
            || self.width > 4096
            || self.height > 4096
            || width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || width * height * 4 > crate::MAX_FRAME_BYTES as u64
            || self.lease_ms == 0
            || self.lease_ms > 1500
        {
            return Err(PresentationError::Invalid);
        }
        Ok(())
    }
    /// Verify host-reported placement against the trusted requested controller/viewport.
    pub fn accepts(&self, lease: Lease) -> bool {
        lease.control_generation == self.control_generation
            && lease.viewport_generation == self.viewport_generation
            && u64::from(lease.pixel_width)
                == (u64::from(self.width) * u64::from(self.scale_milli)).div_ceil(1000)
            && u64::from(lease.pixel_height)
                == (u64::from(self.height) * u64::from(self.scale_milli)).div_ceil(1000)
    }
}

/// Closed human navigation vocabulary; it grants no automation or origin authority.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum HumanCommand {
    /// Navigate within the immutable admitted origin envelope.
    Navigate {
        /// Bounded URL within the immutable host origin envelope.
        url: String,
    },
    /// Go back within the admitted browser context.
    Back,
    /// Go forward within the admitted browser context.
    Forward,
    /// Reload the admitted current document.
    Reload,
    /// Stop loading the admitted current document.
    Stop,
}

/// Private native commands; every effect repeats the exact active lease check.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PresentationCommand {
    /// Authenticated readiness; availability remains supervisor-owned.
    Ready,
    /// Configure an opaque native-owned target.
    Configure(Configure),
    /// Renew an unchanged live visibility lease.
    Renew {
        /// Exact active native lease.
        lease: Lease,
        /// Validated visibility lifetime in milliseconds.
        lease_ms: u16,
    },
    /// Set native focus; keyboard input requires acknowledged focus.
    Focus {
        /// Exact active native lease.
        lease: Lease,
        /// Native focus transition.
        focused: bool,
    },
    /// Revoke visibility and focus.
    Hide {
        /// Exact native lease to hide.
        lease: Lease,
    },
    /// One bounded native input event; no retry after uncertain dispatch.
    Input {
        /// Exact active native lease.
        lease: Lease,
        /// Closed validated native event.
        input: Input,
        /// Native CEF modifier bits, bounded to the accepted mask.
        modifiers: u32,
    },
    /// Read bounded current browser metadata and the fresh opaque document target.
    Observe {
        /// Exact owned lease; metadata remains readable after hide or navigation.
        lease: Lease,
    },
    /// Irrevocably end initial human input before trusted coordinator handoff.
    FenceHuman {
        /// Exact owned human lease; a changed document is recovered in the receipt.
        lease: Lease,
    },
    /// Execute an explicitly human-owned navigation operation.
    Human {
        /// Exact separately admitted human lease.
        lease: Lease,
        /// Closed human navigation operation.
        command: HumanCommand,
    },
    /// Request at most one latest authenticated bounded pixel frame.
    Poll {
        /// Exact active native frame lease.
        lease: Lease,
    },
}
impl PresentationCommand {
    /// Reject oversized or invalid effects before dispatch.
    pub fn validate(&self) -> Result<(), PresentationError> {
        match self {
            Self::Ready => Ok(()),
            Self::Configure(value) => value.validate(),
            Self::Renew { lease, lease_ms } => {
                lease.validate()?;
                if *lease_ms == 0 || *lease_ms > 1500 {
                    Err(PresentationError::Invalid)
                } else {
                    Ok(())
                }
            }
            Self::Input {
                lease,
                input,
                modifiers,
            } => {
                lease.validate()?;
                if *modifiers & !0x1fff != 0 {
                    return Err(PresentationError::Invalid);
                }
                input.validate(lease.pixel_width, lease.pixel_height)
            }
            Self::Human { lease, command } => {
                lease.validate()?;
                if let HumanCommand::Navigate { url } = command
                    && (url.len() > 4096 || url.is_empty() || url.chars().any(char::is_control))
                {
                    return Err(PresentationError::Invalid);
                }
                Ok(())
            }
            Self::Focus { lease, .. }
            | Self::Hide { lease }
            | Self::Observe { lease }
            | Self::Poll { lease } => lease.validate(),
            Self::FenceHuman { lease } => {
                lease.validate()?;
                if lease.control_generation != 0 {
                    Err(PresentationError::Invalid)
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// Bounded native page metadata; no DOM, credentials or raw DevTools payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageState {
    /// Fresh native-owned opaque document identity.
    pub target: BrowserTarget,
    /// Admitted address, bounded to 4096 bytes.
    pub url: String,
    /// Native title, bounded to 2048 bytes.
    pub title: String,
    /// Whether native Chromium is loading.
    pub loading: bool,
    /// Whether a back navigation is currently possible.
    pub can_go_back: bool,
    /// Whether a forward navigation is currently possible.
    pub can_go_forward: bool,
}
impl PageState {
    /// Check the retained native metadata ceiling.
    pub fn validate(&self) -> Result<(), PresentationError> {
        if self.url.len() > 4096 || self.title.len() > 2048 {
            Err(PresentationError::LimitExceeded)
        } else {
            Ok(())
        }
    }
}

/// Authenticated native fence evidence; it never grants agent authority itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanFenceReceipt {
    /// Exact prior native attachment whose initial human authority was revoked.
    pub prior_lease: Lease,
    /// Current opaque document after the native owning thread applied the fence.
    pub state: PageState,
    /// Actual positive native document generation observed on that same thread.
    pub native_document_generation: u64,
}
impl HumanFenceReceipt {
    /// Validate bounded, monotonic native evidence before coordinator adoption.
    pub fn validate(&self) -> Result<(), PresentationError> {
        self.prior_lease.validate()?;
        self.state.validate()?;
        if self.prior_lease.control_generation != 0
            || self.native_document_generation < self.prior_lease.document_generation
        {
            Err(PresentationError::Stale)
        } else {
            Ok(())
        }
    }
}

/// Categorical authenticated native acknowledgment.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PresentationReply {
    /// The exact private endpoint is ready.
    Ready,
    /// Native effect acknowledged.
    Ack,
    /// Fresh configured native frame lease.
    Configured(Lease),
    /// Bounded native metadata.
    State(PageState),
    /// Native human input is irreversibly fenced; coordinator adoption remains separate.
    Fenced(HumanFenceReceipt),
    /// No newer pixel frame is available.
    Empty,
    /// An authenticated binary frame follows this control reply.
    Frame(Lease),
    /// Categorical rejection; no hostile page text appears in diagnostics.
    Error(PresentationError),
}

/// Direction-, enrollment- and sequence-bound control authentication.
pub struct ControlCodec {
    key: Zeroizing<[u8; 32]>,
    enrollment: [u8; 32],
    role: Role,
    sequence: u64,
}
impl ControlCodec {
    /// Construct one exact direction; keys never become ordinary DTO fields.
    pub fn new(key: Zeroizing<[u8; 32]>, enrollment: [u8; 32], role: Role) -> Self {
        Self {
            key,
            enrollment,
            role,
            sequence: 0,
        }
    }
    fn mac(&self, header: &[u8], payload: &[u8]) -> Hmac<Sha256> {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.key.as_ref())
            .unwrap_or_else(|_| unreachable!("fixed HMAC key length"));
        mac.update(b"colossus-browser-presentation-v1\0control\0");
        mac.update(match self.role {
            Role::NativeToHost => b"native-to-host\0",
            Role::HostToNative => b"host-to-native\0",
        });
        mac.update(header);
        mac.update(payload);
        mac
    }
    /// Check the fixed header before allocating the bounded payload.
    pub fn payload_length(
        &self,
        header: &[u8; CONTROL_HEADER_BYTES],
    ) -> Result<usize, PresentationError> {
        if &header[..8] != MAGIC || header[8..40] != self.enrollment {
            return Err(PresentationError::Unauthenticated);
        }
        let length = u32::from_be_bytes(
            header[48..52]
                .try_into()
                .map_err(|_| PresentationError::Invalid)?,
        ) as usize;
        if length == 0 || length > MAX_CONTROL_BYTES {
            return Err(PresentationError::LimitExceeded);
        }
        Ok(length)
    }
    /// Serialize bounded typed control under a fresh monotonic sequence.
    pub fn encode<T: Serialize>(&mut self, value: &T) -> Result<Vec<u8>, PresentationError> {
        let payload = serde_json::to_vec(value).map_err(|_| PresentationError::Invalid)?;
        if payload.is_empty() || payload.len() > MAX_CONTROL_BYTES {
            return Err(PresentationError::LimitExceeded);
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(PresentationError::LimitExceeded)?;
        let mut header = [0_u8; CONTROL_HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8..40].copy_from_slice(&self.enrollment);
        header[40..48].copy_from_slice(&sequence.to_be_bytes());
        header[48..52].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        let tag = self.mac(&header[..52], &payload).finalize().into_bytes();
        header[52..].copy_from_slice(&tag);
        self.sequence = sequence;
        Ok([header.as_slice(), payload.as_slice()].concat())
    }
    /// Authenticate before parsing; a rejected frame never advances the sequence.
    pub fn decode<T: for<'de> Deserialize<'de>>(
        &mut self,
        bytes: &[u8],
    ) -> Result<T, PresentationError> {
        let header: &[u8; CONTROL_HEADER_BYTES] = bytes
            .get(..CONTROL_HEADER_BYTES)
            .ok_or(PresentationError::Invalid)?
            .try_into()
            .map_err(|_| PresentationError::Invalid)?;
        let length = self.payload_length(header)?;
        if bytes.len() != CONTROL_HEADER_BYTES + length {
            return Err(PresentationError::Invalid);
        }
        let sequence = u64::from_be_bytes(
            header[40..48]
                .try_into()
                .map_err(|_| PresentationError::Invalid)?,
        );
        if sequence
            != self
                .sequence
                .checked_add(1)
                .ok_or(PresentationError::LimitExceeded)?
        {
            return Err(PresentationError::Stale);
        }
        self.mac(&header[..52], &bytes[CONTROL_HEADER_BYTES..])
            .verify_slice(&header[52..])
            .map_err(|_| PresentationError::Unauthenticated)?;
        let value = serde_json::from_slice(&bytes[CONTROL_HEADER_BYTES..])
            .map_err(|_| PresentationError::Invalid)?;
        self.sequence = sequence;
        Ok(value)
    }
}
