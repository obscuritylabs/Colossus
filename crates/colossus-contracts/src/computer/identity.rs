use serde::{Deserialize, Deserializer, Serialize, de::Error};
use std::fmt;

/// Invalid native desktop metadata or input. Diagnostics never repeat rejected content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComputerValidationError;

impl fmt::Display for ComputerValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid computer identity, bounds, or input")
    }
}

impl std::error::Error for ComputerValidationError {}

macro_rules! computer_identity {
    ($name:ident, $prefix:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate syntax only; possession does not establish native authority.
            pub fn parse(value: impl Into<String>) -> Result<Self, ComputerValidationError> {
                let value = value.into();
                let suffix = value.strip_prefix($prefix).ok_or(ComputerValidationError)?;
                if suffix.len() != 32
                    || !suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(ComputerValidationError);
                }
                Ok(Self(value))
            }

            /// Borrow the bounded opaque identity.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }
    };
}

computer_identity!(
    ComputerSessionId,
    "cu_s_",
    "Opaque run-owned computer session."
);
computer_identity!(ComputerWindowId, "cu_w_", "Opaque observed native window.");
computer_identity!(
    ComputerObservationId,
    "cu_o_",
    "Opaque single-use native observation."
);
computer_identity!(
    ComputerElementId,
    "cu_e_",
    "Opaque observation-bound semantic element."
);
