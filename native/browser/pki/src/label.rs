use x509_parser::prelude::{FromDer as _, X509Certificate};

use crate::ValidationError;

const MAX_JSON_BYTES: usize = 100;
const FALLBACK: &str = "Client certificate";

/// A bounded public subject label for a native client-identity review.
///
/// Prefer a readable common name, then the distinguished name. Strip control and
/// directional formatting characters, collapse whitespace, and truncate at a UTF-8
/// boundary. JSON string encoding, including its quotes, occupies at most 100 bytes.
/// The result is untrusted plain text, never HTML or proof of identity eligibility;
/// callers must still use [`crate::identity_validity`] and the exact fingerprint.
/// No native key lookup or certificate selection takes place here.
pub fn identity_label(der: &[u8]) -> Result<String, ValidationError> {
    if der.is_empty() || der.len() > 65_536 {
        return Err(ValidationError);
    }
    let (remaining, certificate) = X509Certificate::from_der(der).map_err(|_| ValidationError)?;
    if !remaining.is_empty() {
        return Err(ValidationError);
    }
    for common_name in certificate.subject().iter_common_name() {
        if let Ok(value) = common_name.as_str() {
            let label = display_label(value);
            if !label.is_empty() {
                return Ok(label);
            }
        }
    }
    let label = certificate
        .subject()
        .to_string_with_registry(x509_parser::objects::oid_registry())
        .map(|value| display_label(&value))
        .unwrap_or_default();
    Ok(if label.is_empty() {
        FALLBACK.to_owned()
    } else {
        label
    })
}

fn display_label(value: &str) -> String {
    let mut result = String::with_capacity(MAX_JSON_BYTES - 2);
    let mut json_bytes = 2;
    let mut space = false;
    for character in value.chars() {
        if character.is_control()
            || character.is_whitespace()
            || matches!(
                character,
                '\u{00ad}' | '\u{061c}' | '\u{200b}' | '\u{200e}' | '\u{200f}'
                    | '\u{202a}'..='\u{202e}' | '\u{2060}' | '\u{2066}'..='\u{206f}' | '\u{feff}'
            )
        {
            space = !result.is_empty();
            continue;
        }
        let bytes = json_character_bytes(character);
        if json_bytes + usize::from(space) + bytes > MAX_JSON_BYTES {
            // Leave a visible truncation marker without splitting a code point or
            // exceeding the serialized budget when quotes/backslashes need escaping.
            while json_bytes + '…'.len_utf8() > MAX_JSON_BYTES || result.ends_with(' ') {
                if let Some(removed) = result.pop() {
                    json_bytes -= json_character_bytes(removed);
                }
            }
            result.push('…');
            break;
        }
        if space {
            result.push(' ');
            json_bytes += 1;
            space = false;
        }
        result.push(character);
        json_bytes += bytes;
    }
    result
}

fn json_character_bytes(character: char) -> usize {
    match character {
        '"' | '\\' => 2,
        _ => character.len_utf8(),
    }
}
