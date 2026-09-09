//! Recognize whole credential option names without treating boolean flags as values.

use std::{ops::Range, sync::LazyLock};

use regex::Regex;

static FIELDS: LazyLock<Regex> = LazyLock::new(|| {
    // Match the whole candidate name before classification, including unknown
    // long options with '='. A suffix inside --passthru is not an assignment.
    Regex::new(r#"(?:[\w-]+(?:\\?["'])?\s*[=:]\s*|--[\w-]+\s+)"#)
        .expect("constant display field candidate pattern")
});

pub(super) fn is_option(name: &str) -> bool {
    let Some(name) = name.strip_prefix("--") else {
        return false;
    };
    let mut normalized = String::with_capacity(name.len());
    let mut previous = None;
    for character in name.chars() {
        if character.is_ascii_uppercase()
            && previous
                .is_some_and(|value: char| value.is_ascii_lowercase() || value.is_ascii_digit())
        {
            normalized.push('_');
        }
        normalized.push(if character == '-' {
            '_'
        } else {
            character.to_ascii_lowercase()
        });
        previous = Some(character);
    }
    // Negated options and suffixes such as --password-stdin/--token-type do
    // not carry credentials. Separator/camelCase prefixes remain supported.
    if normalized.starts_with("no_") {
        return false;
    }
    matches!(
        normalized.rsplit('_').next(),
        Some(
            "pass"
                | "password"
                | "passwd"
                | "passphrase"
                | "secret"
                | "token"
                | "authorization"
                | "credential"
                | "cookie"
                | "apikey"
                | "privatekey"
                | "secretkey"
                | "accesskey"
                | "encryptionkey"
                | "signingkey"
                | "authkey"
        )
    ) || [
        "api_key",
        "private_key",
        "secret_key",
        "access_key",
        "encryption_key",
        "signing_key",
        "auth_key",
    ]
    .iter()
    .any(|suffix| normalized == *suffix || normalized.ends_with(&format!("_{suffix}")))
}

pub(super) fn ranges(text: &str, spelling: &str, original_ends: &[usize]) -> Vec<Range<usize>> {
    super::shell_value::filtered_ranges(text, &[&FIELDS], spelling, original_ends, |prefix| {
        let name = prefix
            .split(|character: char| {
                character.is_whitespace() || matches!(character, '=' | ':' | '\\' | '"' | '\'')
            })
            .next()
            .unwrap_or_default();
        if name.starts_with("--") {
            is_option(name) || super::CREDENTIAL_VALUE_OPTIONS.contains(&name)
        } else {
            // Assignment/environment keys retain conservative recognition;
            // they do carry a value, unlike arbitrary boolean option names.
            super::SECRET_FIELDS.is_match(name)
        }
    })
}
