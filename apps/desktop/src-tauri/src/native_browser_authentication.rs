//! Private Desktop key material for the authenticated native browser channel.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use colossus_sdk::Secret;
use zeroize::Zeroizing;

/// Generate the SDK's exact 32-byte visible-ASCII native browser secret.
///
/// The inherited channel re-encodes these bytes as a 32-byte transport key.
/// Twenty-four fresh random bytes yield 32 base64url characters and retain
/// 192 bits of entropy without changing the SDK's credential validator.
pub(crate) fn generate() -> anyhow::Result<Secret> {
    let mut raw = Zeroizing::new([0_u8; 24]);
    getrandom::fill(&mut raw[..])?;
    let encoded = URL_SAFE_NO_PAD.encode(&raw[..]);
    anyhow::ensure!(encoded.len() == 32, "native browser token shape changed");
    Ok(Secret::new(encoded.into_bytes())?)
}

#[cfg(test)]
mod tests {
    use super::generate;

    #[test]
    fn generated_native_authentication_meets_sdk_shape_without_binary_exception() {
        let secret = generate().expect("random native browser authentication");
        assert_eq!(secret.expose().len(), 32);
        assert!(
            secret
                .expose()
                .iter()
                .all(|byte| (0x21..=0x7e).contains(byte))
        );
        assert!(colossus_sdk::Secret::new(vec![0_u8; 32]).is_err());
        assert!(colossus_sdk::Secret::new(vec![b' '; 32]).is_err());
    }
}
