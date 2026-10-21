//! Public display bounds, independent of any retained native request key.

use super::{ClientIdentityDisplayCandidate, unavailable};
use crate::dto::CommandErrorDto;

pub(crate) struct ClientIdentityDisplay {
    pub(crate) origin: String,
    pub(crate) candidates: Vec<ClientIdentityDisplayCandidate>,
    pub(crate) remaining_ms: u32,
}

pub(super) fn validate(display: &ClientIdentityDisplay) -> Result<(), CommandErrorDto> {
    let url = url::Url::parse(&display.origin).map_err(|_| unavailable())?;
    if display.origin.len() > 4096
        || url.scheme() != "https"
        || url.host_str().is_none()
        || url.origin().ascii_serialization() != display.origin
        || display.candidates.is_empty()
        || display.candidates.len() > 64
        || !(1..=120_000).contains(&display.remaining_ms)
    {
        return Err(unavailable());
    }
    let mut seen = std::collections::HashSet::new();
    for candidate in &display.candidates {
        let fingerprint = &candidate.fingerprint_sha256;
        if fingerprint.len() != 64
            || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
            || fingerprint.bytes().any(|byte| byte.is_ascii_uppercase())
            || !seen.insert(fingerprint.as_str())
            || candidate.label.trim().is_empty()
            || candidate.label.len() > 1024
            || candidate.label.chars().count() > 256
            || candidate.label.chars().any(|character| {
                character.is_control()
                    || matches!(character, '\u{00ad}' | '\u{061c}' | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}' | '\u{2066}'..='\u{206f}' | '\u{feff}')
            })
        {
            return Err(unavailable());
        }
    }
    Ok(())
}
