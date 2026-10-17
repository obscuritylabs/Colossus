use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Exact native enrollment generations; a renderer cannot choose this binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    /// Exact owned native tab identifier.
    pub tab: u64,
    /// Owned session generation.
    pub session_generation: u64,
    /// Current single-writer control generation.
    pub control_generation: u64,
    /// Current native presentation generation.
    pub viewport_generation: u64,
    /// Native document generation, changed before each allowed main-frame browse.
    pub document_generation: u64,
    /// Physical frame width in pixels.
    pub pixel_width: u32,
    /// Physical frame height in pixels.
    pub pixel_height: u32,
}
impl Lease {
    /// Reject zero native identities and frames outside the native memory bound.
    /// Control generation zero is reserved for separately admitted human ownership.
    pub fn validate(self) -> Result<(), PresentationError> {
        if [
            self.tab,
            self.session_generation,
            self.viewport_generation,
            self.document_generation,
        ]
        .contains(&0)
            || self.pixel_width == 0
            || self.pixel_height == 0
            || self.pixel_width > 4096
            || self.pixel_height > 4096
            || u64::from(self.pixel_width) * u64::from(self.pixel_height) * 4
                > crate::MAX_FRAME_BYTES as u64
        {
            return Err(PresentationError::Invalid);
        }
        Ok(())
    }
}
/// Categorical errors; no page data or native key appears in diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PresentationError {
    /// Malformed dimensions, bytes or input.
    Invalid,
    /// Ownership or sequence does not match the active lease.
    Stale,
    /// Enrollment or frame authentication failed.
    Unauthenticated,
    /// Native visibility, focus or heartbeat is absent.
    Hidden,
    /// Declared payload exceeds the allocation bound.
    LimitExceeded,
    /// A private channel disconnected after a command could have reached its host.
    OutcomeUnknown,
}

/// Closed native input vocabulary, independent of runtime/model browser commands.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Input {
    /// Native pointer movement.
    MouseMove {
        /// Physical viewport horizontal coordinate.
        x: u32,
        /// Physical viewport vertical coordinate.
        y: u32,
    },
    /// Native mouse button transition.
    MouseButton {
        /// Physical viewport horizontal coordinate.
        x: u32,
        /// Physical viewport vertical coordinate.
        y: u32,
        /// Left0, middle1 or right2 button.
        button: u8,
        /// Whether the key or button is pressed.
        pressed: bool,
    },
    /// Bounded native scroll event.
    MouseWheel {
        /// Physical viewport horizontal coordinate.
        x: u32,
        /// Physical viewport vertical coordinate.
        y: u32,
        /// Bounded horizontal wheel delta.
        delta_x: i32,
        /// Bounded vertical wheel delta.
        delta_y: i32,
    },
    /// Native key transition.
    Key {
        /// Native adapter mapped virtual key code.
        code: u16,
        /// Whether the key or button is pressed.
        pressed: bool,
    },
    /// One completed Unicode character.
    Character {
        /// Completed Unicode input text.
        text: String,
    },
    /// Completed IME text, bounded to4096UTF16units.
    ImeCommit {
        /// Completed Unicode input text.
        text: String,
    },
    /// Cancel active input composition.
    ImeCancel,
}
impl Input {
    /// Reject off-viewport coordinates and malformed or oversized text.
    pub fn validate(&self, width: u32, height: u32) -> Result<(), PresentationError> {
        let point = |x, y| x < width && y < height;
        let valid = match self {
            Self::MouseMove { x, y } => point(*x, *y),
            Self::MouseButton { x, y, button, .. } => point(*x, *y) && *button <= 2,
            Self::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                point(*x, *y)
                    && (-4096..=4096).contains(delta_x)
                    && (-4096..=4096).contains(delta_y)
            }
            Self::Key { code, .. } => *code != 0,
            Self::Character { text } => text.chars().count() == 1 && !text.contains('\0'),
            Self::ImeCommit { text } => {
                !text.is_empty() && text.encode_utf16().count() <= 4096 && !text.contains('\0')
            }
            Self::ImeCancel => true,
        };
        if valid {
            Ok(())
        } else {
            Err(PresentationError::Invalid)
        }
    }
}
/// Hide, focus loss, heartbeat expiration and lease replacement deny fresh input.
pub struct LeaseGuard {
    lease: Lease,
    deadline: Instant,
    visible: bool,
    focused: bool,
}
impl LeaseGuard {
    /// Install a validated native visibility lease without keyboard focus.
    pub fn new(lease: Lease, now: Instant, ttl: Duration) -> Result<Self, PresentationError> {
        lease.validate()?;
        if ttl.is_zero() || ttl > Duration::from_millis(1500) {
            return Err(PresentationError::Invalid);
        }
        Ok(Self {
            lease,
            deadline: now + ttl,
            visible: true,
            focused: false,
        })
    }
    /// Return the exact trusted native binding.
    pub fn lease(&self) -> Lease {
        self.lease
    }
    /// Renew a live unchanged lease; expiration requires fresh presentation ownership.
    pub fn renew(
        &mut self,
        lease: Lease,
        now: Instant,
        ttl: Duration,
    ) -> Result<(), PresentationError> {
        if lease != self.lease {
            return Err(PresentationError::Stale);
        }
        if !self.visible(now) || ttl.is_zero() || ttl > Duration::from_millis(1500) {
            self.hide();
            return Err(PresentationError::Hidden);
        }
        self.deadline = now + ttl;
        Ok(())
    }
    /// Whether visibility and heartbeat remain live.
    pub fn visible(&self, now: Instant) -> bool {
        self.visible && now < self.deadline
    }
    /// Change native focus only while the surface is visible.
    pub fn focus(&mut self, focused: bool, now: Instant) -> Result<(), PresentationError> {
        if focused && !self.visible(now) {
            return Err(PresentationError::Hidden);
        }
        self.focused = focused;
        Ok(())
    }
    /// Revoke visibility and keyboard focus immediately.
    pub fn hide(&mut self) {
        self.visible = false;
        self.focused = false;
    }
    /// Validate exact binding, live focus and bounded native input.
    pub fn authorize_input(
        &self,
        lease: Lease,
        input: &Input,
        now: Instant,
    ) -> Result<(), PresentationError> {
        if lease != self.lease {
            return Err(PresentationError::Stale);
        }
        if !self.visible(now) || !self.focused {
            return Err(PresentationError::Hidden);
        }
        input.validate(lease.pixel_width, lease.pixel_height)
    }
}
