use std::io;

/// Fixed helper front selected from native verified allocation metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HelperRole {
    /// Renderer front and immutable renderer policy.
    Renderer = 1,
    /// GPU front and immutable GPU policy.
    Gpu = 2,
    /// Network-service front and immutable network policy.
    Network = 3,
    /// General utility front and immutable utility policy.
    Utility = 4,
}

impl HelperRole {
    pub(crate) fn decode(value: u8) -> io::Result<Self> {
        match value {
            1 => Ok(Self::Renderer),
            2 => Ok(Self::Gpu),
            3 => Ok(Self::Network),
            4 => Ok(Self::Utility),
            _ => Err(invalid("unknown owned helper role")),
        }
    }
}

/// Bounded, reviewed Chromium switches, without a program or path authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpawnArguments {
    role: HelperRole,
    values: Vec<String>,
}

impl SpawnArguments {
    /// Reject unknown switches and authority-changing launch configuration.
    ///
    /// This initial source increment deliberately supports only renderer, GPU,
    /// network and general utility launch switches. Missing Chromium switches
    /// fail closed and require review before extending this table.
    pub fn new(role: HelperRole, values: Vec<String>) -> io::Result<Self> {
        if values.is_empty() || values.len() > 128 {
            return Err(invalid("owned helper argument count is out of bounds"));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut process = None;
        let mut service = None;
        for argument in &values {
            if argument.len() > 4096 || !argument.bytes().all(|b| (32..=126).contains(&b)) {
                return Err(invalid("owned helper argument is out of bounds"));
            }
            let argument = argument
                .strip_prefix("--")
                .ok_or_else(|| invalid("owned helper expects switches"))?;
            let (name, value) = argument
                .split_once('=')
                .map_or((argument, None), |(n, v)| (n, Some(v)));
            if !seen.insert(name) {
                return Err(invalid("duplicate owned helper switch"));
            }
            match name {
                "type" => process = value,
                "service-sandbox-type" => service = value,
                "lang" => {
                    if !value.is_some_and(|v| {
                        !v.is_empty()
                            && v.len() <= 32
                            && v.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')
                    }) {
                        return Err(invalid("invalid helper language"));
                    }
                }
                "renderer-client-id"
                | "num-raster-threads"
                | "time-ticks-at-unix-epoch"
                | "launch-time-ticks" => {
                    if !value.is_some_and(decimal) {
                        return Err(invalid("invalid helper numeric switch"));
                    }
                }
                "field-trial-handle"
                | "variations-seed-version"
                | "gpu-preferences"
                | "enable-features"
                | "disable-features" => {
                    if !value.is_some_and(|v| !v.is_empty() && v.len() <= 4096 && !v.contains('/'))
                    {
                        return Err(invalid("invalid helper feature switch"));
                    }
                }
                "shared-files" => {
                    if value != Some("") {
                        return Err(invalid("caller shared files are forbidden"));
                    }
                }
                "enable-main-frame-before-activation"
                | "disable-gpu-compositing"
                | "disable-gpu-vsync"
                | "use-gl"
                | "use-angle" => {
                    if value.is_some()
                        && !matches!(
                            (name, value),
                            ("use-gl", Some("angle")) | ("use-angle", Some("metal"))
                        )
                    {
                        return Err(invalid("invalid helper boolean switch"));
                    }
                }
                _ => return Err(invalid("unreviewed owned helper switch")),
            }
        }
        let valid = match role {
            HelperRole::Renderer => process == Some("renderer") && service.is_none(),
            HelperRole::Gpu => process == Some("gpu-process") && service.is_none(),
            HelperRole::Network => process == Some("utility") && service == Some("network"),
            HelperRole::Utility => process == Some("utility") && service == Some("utility"),
        };
        if !valid {
            return Err(invalid("helper role and arguments disagree"));
        }
        Ok(Self { role, values })
    }
    /// Immutable role selected by the broker.
    pub fn role(&self) -> HelperRole {
        self.role
    }
    /// Validated switches, excluding argv zero and all native authority.
    pub fn values(&self) -> &[String] {
        &self.values
    }
}

fn decimal(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 20
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}
pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
