use colossus_contracts::MAX_HOST_SECRET_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputError {
    Empty,
    TooLong,
    InvalidCharacter,
    InvalidPasswordCharacter,
    PasswordTooLong,
}

impl InputError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Empty => "Enter a token to continue.",
            Self::TooLong => "Token exceeds 65,536 bytes. The change was not accepted.",
            Self::InvalidCharacter => {
                "Use a token without spaces, line breaks, or non-ASCII characters."
            }
            Self::InvalidPasswordCharacter => {
                "Use a passphrase without NUL, line breaks, or control characters."
            }
            Self::PasswordTooLong => {
                "Passphrase exceeds 65,536 UTF-8 bytes. The change was not accepted."
            }
        }
    }
}

pub(crate) fn validate_password(value: &str) -> Result<(), InputError> {
    if value.len() > MAX_HOST_SECRET_BYTES {
        return Err(InputError::PasswordTooLong);
    }
    if value.chars().any(password_control) {
        return Err(InputError::InvalidPasswordCharacter);
    }
    Ok(())
}

fn password_control(character: char) -> bool {
    character.is_control() || matches!(character, '\u{2028}' | '\u{2029}')
}

/// Inspect native UTF-16 without a plaintext Rust String allocation. A Windows
/// single-character edit may temporarily contain a high surrogate until the
/// paired low surrogate arrives; final acceptance always requires valid UTF-16.
#[cfg(any(windows, target_os = "macos", test))]
pub(crate) fn password_units_bytes(
    units: impl IntoIterator<Item = u16>,
    unit_count: usize,
    partial: bool,
) -> Result<usize, InputError> {
    if unit_count > MAX_HOST_SECRET_BYTES {
        return Err(InputError::PasswordTooLong);
    }
    let mut bytes = 0_usize;
    for character in char::decode_utf16(units) {
        let length = match character {
            Ok(character) if !password_control(character) => character.len_utf8(),
            Err(error) if partial && (0xd800..=0xdbff).contains(&error.unpaired_surrogate()) => 3,
            _ => return Err(InputError::InvalidPasswordCharacter),
        };
        bytes = bytes.saturating_add(length);
        if bytes > MAX_HOST_SECRET_BYTES {
            return Err(InputError::PasswordTooLong);
        }
    }
    Ok(bytes)
}

pub(crate) fn validate(value: &str) -> Result<(), InputError> {
    if value.is_empty() {
        return Err(InputError::Empty);
    }
    validate_units(value.bytes().map(u16::from), value.len())
}

/// Check native UTF-16 input before copying/inserting it. Visible ASCII uses one
/// byte and one UTF-16 unit, so the shared byte bound is exact, not an estimate.
pub(crate) fn validate_units(
    units: impl IntoIterator<Item = u16>,
    resulting_len: usize,
) -> Result<(), InputError> {
    if resulting_len > MAX_HOST_SECRET_BYTES {
        return Err(InputError::TooLong);
    }
    if !units
        .into_iter()
        .all(|unit| (u16::from(b'!')..=u16::from(b'~')).contains(&unit))
    {
        return Err(InputError::InvalidCharacter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_long_tokens_are_accepted_and_overflow_is_never_shortened() {
        for length in [761, 762, 2_560, 2_561, 8_192, 65_536] {
            let token = format!("{}END", "X".repeat(length - 3));
            assert_eq!(validate(&token), Ok(()));
            assert_eq!(token.len(), length);
            assert!(token.ends_with("END"));
        }
        assert_eq!(validate(&"X".repeat(65_537)), Err(InputError::TooLong));
    }

    #[test]
    fn manual_entry_preserves_strict_opaque_token_grammar() {
        assert_eq!(validate(""), Err(InputError::Empty));
        for input in [" token", "token ", "token\n", "tok\0en", "tökén", "token\t"] {
            assert_eq!(validate(input), Err(InputError::InvalidCharacter));
        }
        assert_eq!(validate("opaque.+/=._-TOKEN"), Ok(()));
        assert_eq!(
            validate_units([u16::from(b'X')], 65_537),
            Err(InputError::TooLong)
        );
        assert_eq!(
            validate_units([0xd800], 1),
            Err(InputError::InvalidCharacter)
        );
    }

    #[test]
    fn native_passwords_preserve_empty_spaces_unicode_and_exact_utf8_limit() {
        for password in ["", "two words", " 密碼 🔐 café "] {
            assert_eq!(validate_password(password), Ok(()));
            assert_eq!(
                password_units_bytes(
                    password.encode_utf16(),
                    password.encode_utf16().count(),
                    false
                ),
                Ok(password.len())
            );
            let owned = crate::NativePassword::new(password.to_owned()).unwrap();
            assert_eq!(owned.expose(), password);
            assert_eq!(format!("{owned:?}"), "NativePassword([REDACTED])");
        }
        let exact = "🔐".repeat(MAX_HOST_SECRET_BYTES / 4);
        assert_eq!(validate_password(&exact), Ok(()));
        assert_eq!(
            password_units_bytes(exact.encode_utf16(), exact.encode_utf16().count(), false),
            Ok(MAX_HOST_SECRET_BYTES)
        );
        let oversized = format!("{exact}x");
        assert_eq!(
            validate_password(&oversized),
            Err(InputError::PasswordTooLong)
        );
        assert_eq!(
            password_units_bytes(
                oversized.encode_utf16(),
                oversized.encode_utf16().count(),
                false
            ),
            Err(InputError::PasswordTooLong)
        );
        // Password support does not change the existing token contract.
        assert_eq!(validate("two words"), Err(InputError::InvalidCharacter));
        assert_eq!(validate("密碼"), Err(InputError::InvalidCharacter));
        assert_eq!(validate(""), Err(InputError::Empty));
    }

    #[test]
    fn passwords_reject_controls_and_malformed_final_utf16_without_normalizing() {
        for password in ["a\0b", "a\nb", "a\rb", "a\tb", "a\u{2028}b", "a\u{2029}b"] {
            assert_eq!(
                validate_password(password),
                Err(InputError::InvalidPasswordCharacter)
            );
        }
        assert_eq!(
            password_units_bytes([0xd800], 1, false),
            Err(InputError::InvalidPasswordCharacter)
        );
        assert_eq!(
            password_units_bytes([0xdc00], 1, true),
            Err(InputError::InvalidPasswordCharacter)
        );
        assert_eq!(password_units_bytes([0xd800], 1, true), Ok(3));
        assert_eq!(password_units_bytes([0xd83d, 0xdd10], 2, false), Ok(4));
        let decomposed = "e\u{301}";
        assert_eq!(
            crate::NativePassword::new(decomposed.into())
                .unwrap()
                .expose(),
            decomposed
        );
    }
}
