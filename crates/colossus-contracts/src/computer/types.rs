use super::{
    ComputerElementId, ComputerObservationId, ComputerSessionId, ComputerValidationError,
    ComputerWindowId,
};
use serde::{Deserialize, Deserializer, Serialize, de::Error};

/// Authenticated application scope, reconstructed from registered run provenance.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerScope {
    /// One owned conversation.
    Conversation {
        /// Trusted conversation identity.
        id: String,
    },
    /// One owned workflow execution.
    Workflow {
        /// Trusted workflow execution identity.
        id: String,
    },
}

/// Trusted native owner. Never accept these fields from model or renderer arguments.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerBinding {
    /// Supervised runtime instance identity.
    pub runtime_id: String,
    /// Canonical opaque workspace identity, never a filesystem path.
    pub workspace_id: String,
    /// Authenticated application owner.
    pub application_id: String,
    /// Authenticated conversation or workflow scope.
    pub scope: ComputerScope,
    /// Registered run identity; delegated runs require independent authority.
    pub run_id: String,
}

impl ComputerBinding {
    /// Check bounded metadata syntax; ownership still requires trusted provenance.
    pub fn validate(&self) -> Result<(), ComputerValidationError> {
        let scope = match &self.scope {
            ComputerScope::Conversation { id } | ComputerScope::Workflow { id } => id,
        };
        if [
            &self.runtime_id,
            &self.workspace_id,
            &self.application_id,
            &self.run_id,
            scope,
        ]
        .iter()
        .all(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        {
            Ok(())
        } else {
            Err(ComputerValidationError)
        }
    }
}

macro_rules! computer_limits {
    ($($(#[$doc:meta])* $field:ident: $ty:ty = $default:literal, $maximum:literal);* $(;)?) => {
        /// Positive native resource bounds, narrowed independently by runtime policy.
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
        pub struct ComputerLimits { $($(#[$doc])* pub $field: $ty),* }

        impl ComputerLimits {
            /// Absolute protocol ceilings, not default authority.
            pub const HARD_MAX: Self = Self { $($field: $maximum),* };

            /// Reject zero and values beyond fixed protocol ceilings.
            pub fn validate(&self) -> Result<(), ComputerValidationError> {
                if $(self.$field > 0 && self.$field <= Self::HARD_MAX.$field)&&* {
                    Ok(())
                } else { Err(ComputerValidationError) }
            }

            /// Intersect two validated envelopes; this operation never expands either.
            pub fn intersect(&self, other: &Self) -> Result<Self, ComputerValidationError> {
                self.validate()?;
                other.validate()?;
                Ok(Self { $($field: self.$field.min(other.$field)),* })
            }

            /// Whether these valid limits fit entirely inside another valid envelope.
            pub fn is_within(&self, other: &Self) -> bool {
                self.validate().is_ok() && other.validate().is_ok()
                    && $(self.$field <= other.$field)&&*
            }
        }

        impl Default for ComputerLimits {
            fn default() -> Self { Self { $($field: $default),* } }
        }

        impl<'de> Deserialize<'de> for ComputerLimits {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Limits { $($field: $ty),* }
                let wire = Limits::deserialize(deserializer)?;
                let limits = Self { $($field: wire.$field),* };
                limits.validate().map_err(D::Error::custom)?;
                Ok(limits)
            }
        }
    };
}

computer_limits! {
    /// Maximum top-level window choices.
    max_windows: u16 = 64, 128;
    /// Maximum nodes in a semantic observation.
    max_snapshot_nodes: u16 = 512, 1024;
    /// Maximum privately transferred PNG bytes.
    max_screenshot_bytes: u32 = 8388608, 16777216;
    /// Maximum captured physical pixels.
    max_screenshot_pixels: u32 = 8388608, 16777216;
    /// Absolute per-action response deadline in milliseconds.
    action_timeout_ms: u32 = 5000, 10000;
    /// Nonrenewable session lifetime in milliseconds.
    session_timeout_ms: u32 = 300000, 900000;
    /// Maximum age of a consumable observation in milliseconds.
    observation_max_age_ms: u32 = 10000, 30000;
}

/// Native availability evidence, never permission to control another application.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerCapabilities {
    /// Verified normal-user interactive host is available.
    pub available: bool,
    /// Bounded platform category, normally `windows`.
    pub platform: String,
    /// Verified resource ceilings.
    pub limits: ComputerLimits,
    /// Bounded categorical reason; no native pointers, paths, or provider messages.
    pub unavailable_reason: Option<String>,
}

/// Physical rectangle. Window placement may be negative; observation bounds are local.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerRect {
    /// Horizontal origin in the coordinate space documented by its owner.
    pub x: i32,
    /// Vertical origin in the coordinate space documented by its owner.
    pub y: i32,
    /// Positive physical width.
    pub width: u32,
    /// Positive physical height.
    pub height: u32,
}

impl ComputerRect {
    /// Reject empty dimensions or arithmetic overflow while retaining negative monitor origins.
    pub fn validate(&self) -> Result<(), ComputerValidationError> {
        if self.width > 0
            && self.height > 0
            && i32::try_from(i64::from(self.x) + i64::from(self.width)).is_ok()
            && i32::try_from(i64::from(self.y) + i64::from(self.height)).is_ok()
        {
            Ok(())
        } else {
            Err(ComputerValidationError)
        }
    }
}

/// Bounded untrusted native window metadata; no handles, process IDs, or paths.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerWindowSummary {
    /// Opaque native choice, separately bound to this owner.
    pub window_id: ComputerWindowId,
    /// Untrusted title, at most 512 UTF-8 bytes.
    pub title: String,
    /// Display-only application name, at most 128 UTF-8 bytes.
    pub application_name: String,
    /// Current desktop placement metadata, never input authority.
    pub rect: ComputerRect,
    /// Whether this exact window was foreground at observation time.
    pub foreground: bool,
    /// Whether normal-user control is admitted by the native boundary.
    pub available: bool,
    /// Bounded categorical reason; never a raw operating-system error.
    pub unavailable_reason: Option<String>,
}

impl ComputerWindowSummary {
    /// Check metadata ceilings before quarantine or model release.
    pub fn validate(&self) -> Result<(), ComputerValidationError> {
        self.rect.validate()?;
        if self.title.len() <= 512
            && self.application_name.len() <= 128
            && self
                .unavailable_reason
                .as_ref()
                .is_none_or(|reason| reason.len() <= 128)
        {
            Ok(())
        } else {
            Err(ComputerValidationError)
        }
    }
}

/// Native writer state; only trusted human UI can resume paused authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerControlState {
    /// Current run owns one native writer lease.
    Active,
    /// Human pause or takeover invalidated observations and pending input.
    Paused,
    /// Authority is permanently fenced for this run.
    Stopped,
}

/// Categorical reason for authority revocation, without private native details.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerStopReason {
    /// Human pressed Pause.
    UserPaused,
    /// Human pressed Stop.
    UserStopped,
    /// Physical human input interrupted automation.
    UserTakeover,
    /// Nonrenewable session deadline elapsed.
    Expired,
    /// Selected native window no longer exists with the admitted identity.
    WindowClosed,
    /// Interactive input desktop is unavailable or changed.
    DesktopUnavailable,
    /// Trusted operator disabled the feature.
    Disabled,
    /// Agent released automation, leaving the user's app running.
    AgentClosed,
}

/// Owner-bound current native control epoch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerSessionSummary {
    /// Opaque current session.
    pub session_id: ComputerSessionId,
    /// Positive generation, advanced by native authority changes.
    pub control_generation: u64,
    /// Selected root window; captured owned modals have their own observation window ID.
    pub window_id: ComputerWindowId,
    /// Current native writer state.
    pub state: ComputerControlState,
    /// Native categorical explanation when control is paused or stopped.
    pub reason: Option<ComputerStopReason>,
}

impl ComputerSessionSummary {
    /// Reject impossible control epochs and contradictory native state before release.
    pub fn validate(&self) -> Result<(), ComputerValidationError> {
        if self.control_generation > 0
            && (self.state == ComputerControlState::Active) == self.reason.is_none()
        {
            Ok(())
        } else {
            Err(ComputerValidationError)
        }
    }
}

/// Sanitized native semantic node. Password values are always absent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerNode {
    /// Opaque current-observation token, never a UIA pointer.
    pub element_id: ComputerElementId,
    /// Bounded native role, at most 64 UTF-8 bytes.
    pub role: String,
    /// Untrusted accessible name, at most 512 UTF-8 bytes.
    pub name: String,
    /// Ordinary bounded value, absent for protected content.
    pub value: Option<String>,
    /// Local physical bounds in the actual captured window.
    pub bounds: ComputerRect,
    /// Native element enabled state at observation time.
    pub enabled: bool,
    /// Native focus state at observation time.
    pub focused: bool,
    /// Protected input marker; native adapters must withhold its value.
    pub is_password: bool,
}

/// Quarantined semantic observation, released only after runtime policy approval.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerObservation {
    /// Exact native owned session.
    pub session_id: ComputerSessionId,
    /// Current positive native control epoch.
    pub control_generation: u64,
    /// Fresh consumable observation, valid only after runtime release.
    pub observation_id: ComputerObservationId,
    /// Actual selected root or positively owned same-app modal observed.
    pub window_id: ComputerWindowId,
    /// Captured local physical width.
    pub width: u32,
    /// Captured local physical height.
    pub height: u32,
    /// Native DPI scale multiplied by 1000.
    pub scale_milli: u32,
    /// Bounded native semantics with protected content withheld.
    pub nodes: Vec<ComputerNode>,
    /// Only the selected root and positively owned same-app modals.
    pub windows: Vec<ComputerWindowSummary>,
}

impl ComputerObservation {
    /// Validate bounds and protected-value suppression before any semantic release.
    pub fn validate(&self, limits: &ComputerLimits) -> Result<(), ComputerValidationError> {
        limits.validate()?;
        validate_dimensions(self.width, self.height, limits)?;
        if self.control_generation == 0
            || !(1..=8000).contains(&self.scale_milli)
            || self.nodes.len() > usize::from(limits.max_snapshot_nodes)
            || self.windows.len() > usize::from(limits.max_windows)
        {
            return Err(ComputerValidationError);
        }
        for node in &self.nodes {
            node.bounds.validate()?;
            if node.role.len() > 64
                || node.name.len() > 512
                || node
                    .value
                    .as_ref()
                    .is_some_and(|value| value.len() > 512 || node.is_password)
                || node.bounds.x < 0
                || node.bounds.y < 0
                || u64::from(node.bounds.x.unsigned_abs()) + u64::from(node.bounds.width)
                    > u64::from(self.width)
                || u64::from(node.bounds.y.unsigned_abs()) + u64::from(node.bounds.height)
                    > u64::from(self.height)
            {
                return Err(ComputerValidationError);
            }
        }
        self.windows
            .iter()
            .try_for_each(ComputerWindowSummary::validate)
    }
}

/// Bounded private PNG transfer metadata; image bytes never become JSON arrays.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerScreenshotDescriptor {
    /// Exact owned native session.
    pub session_id: ComputerSessionId,
    /// Native authority generation at capture.
    pub control_generation: u64,
    /// Fresh native observation, released only with the validated PNG.
    pub observation_id: ComputerObservationId,
    /// Actual root or positively owned same-app modal captured.
    pub window_id: ComputerWindowId,
    /// Physical PNG width.
    pub width: u32,
    /// Physical PNG height.
    pub height: u32,
    /// Exact binary transfer length.
    pub size_bytes: u32,
    /// Lowercase SHA-256 of actual PNG bytes, verified independently by the runtime.
    pub sha256: String,
}

impl ComputerScreenshotDescriptor {
    /// Validate declared bounds; the runtime must separately decode and hash actual PNG bytes.
    pub fn validate(&self, limits: &ComputerLimits) -> Result<(), ComputerValidationError> {
        limits.validate()?;
        validate_dimensions(self.width, self.height, limits)?;
        if self.control_generation > 0
            && self.size_bytes > 0
            && self.size_bytes <= limits.max_screenshot_bytes
            && self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(())
        } else {
            Err(ComputerValidationError)
        }
    }
}

fn validate_dimensions(
    width: u32,
    height: u32,
    limits: &ComputerLimits,
) -> Result<(), ComputerValidationError> {
    if width > 0
        && height > 0
        && u64::from(width) * u64::from(height) <= u64::from(limits.max_screenshot_pixels)
    {
        Ok(())
    } else {
        Err(ComputerValidationError)
    }
}
