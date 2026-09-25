use colossus_contracts::MAX_HOST_SECRET_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputError {
    Empty,
    TooLong,
    InvalidCharacter,
}

impl InputError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Empty => "Enter a token to continue.",
            Self::TooLong => "Token exceeds 65,536 bytes. The change was not accepted.",
            Self::InvalidCharacter => {
                "Use a token without spaces, line breaks, or non-ASCII characters."
            }
        }
    }
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
}
