use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};
use std::fmt;

/// Invalid browser handle, URL, or origin. Diagnostics contain no rejected input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserValidationError;

impl fmt::Display for BrowserValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid browser identity or destination")
    }
}

impl std::error::Error for BrowserValidationError {}

macro_rules! handle {
    ($name:ident, $prefix:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate an opaque handle; this does not establish ownership or authority.
            pub fn parse(value: impl Into<String>) -> Result<Self, BrowserValidationError> {
                let value = value.into();
                let suffix = value.strip_prefix($prefix).ok_or(BrowserValidationError)?;
                if suffix.len() != 32
                    || !suffix
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(BrowserValidationError);
                }
                Ok(Self(value))
            }

            /// Borrow the opaque identifier without exposing engine state.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }
    };
}

handle!(
    BrowserSessionId,
    "bs_",
    "Opaque browser session identifier."
);
handle!(BrowserTabId, "bt_", "Opaque browser tab identifier.");
handle!(
    BrowserProfileId,
    "bp_",
    "Opaque workspace profile identifier; possession never establishes ownership."
);
handle!(
    BrowserDocumentId,
    "bd_",
    "Opaque document identity, replaced after navigation."
);
handle!(
    BrowserSnapshotId,
    "bn_",
    "Opaque snapshot identity, invalidated after page actions."
);
handle!(
    BrowserElementId,
    "be_",
    "Opaque element token meaningful only within its snapshot."
);
handle!(
    BrowserControlLeaseId,
    "bl_",
    "Opaque control lease identifier; possession grants no authority."
);

fn web_url(value: &str) -> Result<url::Url, BrowserValidationError> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(BrowserValidationError);
    }
    let parsed = url::Url::parse(value).map_err(|_| BrowserValidationError)?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || parsed.host_str().is_some_and(|host| host.contains('*'))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(BrowserValidationError);
    }
    Ok(parsed)
}

/// Validated HTTP(S) destination. Do not log this value: paths and queries can be sensitive.
#[derive(Clone, Eq, PartialEq)]
pub struct BrowserUrl {
    value: String,
    origin: BrowserOrigin,
}

impl fmt::Debug for BrowserUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BrowserUrl").field(&self.origin()).finish()
    }
}

impl BrowserUrl {
    /// Parse an HTTP(S) URL, rejecting credentials, control characters, and excessive length.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, BrowserValidationError> {
        let parsed = web_url(value.as_ref())?;
        Ok(Self {
            origin: BrowserOrigin(parsed.origin().ascii_serialization()),
            value: parsed.to_string(),
        })
    }

    /// Native driver destination; callers must not include it in ordinary diagnostics.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Canonical scheme, host, and effective port, without path, query, or fragment.
    pub fn origin(&self) -> BrowserOrigin {
        self.origin.clone()
    }
}

impl Serialize for BrowserUrl {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.value)
    }
}

impl<'de> Deserialize<'de> for BrowserUrl {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// Exact canonical origin. Importing trust or identity never authorizes this destination.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize)]
#[serde(transparent)]
pub struct BrowserOrigin(String);

impl BrowserOrigin {
    /// Accept an exact HTTP(S) origin, with only an optional trailing slash.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, BrowserValidationError> {
        let parsed = web_url(value.as_ref())?;
        if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(BrowserValidationError);
        }
        Ok(Self(parsed.origin().ascii_serialization()))
    }

    /// Canonical origin suitable for policy comparisons and display.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BrowserOrigin {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}
