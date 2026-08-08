use std::{cmp::Ordering, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SemanticVersion {
    major: u64,
    minor: u64,
    patch: u64,
    preview: Option<u64>,
}

impl SemanticVersion {
    pub(crate) fn is_stable(self) -> bool {
        self.preview.is_none()
    }
}

impl FromStr for SemanticVersion {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.starts_with('v')
            || value.bytes().any(|byte| byte.is_ascii_whitespace())
        {
            return Err(());
        }
        let (core, preview) = match value.split_once("-preview.") {
            Some((core, preview)) => (core, Some(parse_number(preview)?)),
            None if !value.contains('-') => (value, None),
            None => return Err(()),
        };
        let mut parts = core.split('.');
        let major = parse_number(parts.next().ok_or(())?)?;
        let minor = parse_number(parts.next().ok_or(())?)?;
        let patch = parse_number(parts.next().ok_or(())?)?;
        if parts.next().is_some() || preview == Some(0) {
            return Err(());
        }
        Ok(Self {
            major,
            minor,
            patch,
            preview,
        })
    }
}

impl Ord for SemanticVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.preview, other.preview) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => left.cmp(&right),
            })
    }
}

impl PartialOrd for SemanticVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_number(value: &str) -> Result<u64, ()> {
    if value.is_empty() || value.len() > 1 && value.starts_with('0') {
        return Err(());
    }
    value.parse().map_err(|_| ())
}
