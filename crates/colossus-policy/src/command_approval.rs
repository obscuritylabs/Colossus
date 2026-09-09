//! One credential-free display projection; never changes executable input.

use std::sync::LazyLock;

use colossus_contracts::{CommandApprovalContext, EffectRequest, unsafe_command_display_character};
use regex::Regex;

use crate::GatewayError;

mod credential_options;
mod shell_value;

const REDACTED: &str = "[REDACTED]";
// Stop at the first value-taking credential option in a short-option group.
// Later letters are the attached value, not more flags to inspect.
const SHORT_CREDENTIAL_FLAG: &str = r"-[a-zA-Z#]*?[uUbHE]";
// Shared by shell spelling and argv recognition. Certificate options can carry
// an attached passphrase, so their entire value is private display data too.
const CREDENTIAL_VALUE_OPTIONS: &[&str] = &[
    "--user",
    "--proxy-user",
    "--oauth2-bearer",
    "--cookie",
    "--header",
    "--proxy-header",
    "--tlsuser",
    "--proxy-tlsuser",
    "--pass",
    "--proxy-pass",
    "--cert",
    "--proxy-cert",
    "--login-options",
    "-passin",
    "-passout",
    "-pw",
];
const SECRET_FIELD_PATTERN: &str = r"pass(?:word|wd|phrase)?|secret|token|api[-_]?key|authorization|credential|private[-_]?key|cookie";
static SHORT_CREDENTIAL_OPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("^{SHORT_CREDENTIAL_FLAG}"))
        .expect("constant short credential option pattern")
});

static SECRET_FIELDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("(?i)({SECRET_FIELD_PATTERN})")).expect("constant credential field pattern")
});
static SECRET_ASSIGNMENTS: LazyLock<Regex> = LazyLock::new(|| {
    let field = format!(r"[\w-]*(?:{SECRET_FIELD_PATTERN})[\w-]*");
    Regex::new(&format!(
        r#"(?i)(?:{field}(?:\\?["'])?\s*[=:]\s*|--{field}\s+)"#
    ))
    .expect("constant credential assignment pattern")
});
static AUTHORIZATION: LazyLock<Regex> = LazyLock::new(|| {
    // Consume the whole credential word, not merely a partial token alphabet.
    Regex::new(r"(?i)\b(?:bearer|basic)\s+").expect("constant authorization pattern")
});
static CREDENTIAL_HEADERS: LazyLock<Regex> = LazyLock::new(|| {
    // Headers are often one quoted shell word or one argv element. Remove the
    // whole value, including all cookie pairs, rather than just its first token.
    Regex::new(r#"(?i)((?:\b|-[a-zA-Z#]*?H)(?:cookie|set-cookie|authorization|proxy-authorization|x-api-key)\s*:\s*)[^\r\n"']+"#)
        .expect("constant credential header pattern")
});
static URL_USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    // WHATWG consumers accept unescaped @ inside passwords. Greedily mask to
    // the last @ in this authority, never into a path, query or fragment.
    Regex::new(r#"(?i)([a-z][a-z0-9+.-]*://)[^\s/?#"'`\\]*@"#)
        .expect("constant URL credential pattern")
});
static CREDENTIAL_FLAGS: LazyLock<Regex> = LazyLock::new(|| {
    // Treat complete header-option payloads as private too: shell quoting may
    // split the credential header name, and display must never evaluate it.
    let options = CREDENTIAL_VALUE_OPTIONS
        .iter()
        .map(|name| regex::escape(name))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(
        r"(?:^|[\s;&|])(?:(?:{options})[\s=]+|{SHORT_CREDENTIAL_FLAG}[\s=]*)",
    ))
    .expect("constant credential flag pattern")
});
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    // OpenPGP armor has a distinct PRIVATE KEY BLOCK label. A PEM footer
    // cannot terminate it; malformed or missing armor footers mask to EOF.
    Regex::new(concat!(
        r"(?s)-----BEGIN PGP PRIVATE KEY BLOCK-----.*?(?:-----END PGP PRIVATE KEY BLOCK-----|$)",
        r"|-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)",
    ))
    .expect("constant private key pattern")
});

/// Release only prepared `shell.run` command details with validated task intent.
///
/// Legacy requests have no context. This is not a general effect disclosure API.
/// Known credential environment values and recognizable inline credentials are
/// removed without reading any credential store. Unknown arbitrary secrets cannot
/// be inferred: callers must never put credentials into task explanations.
pub fn command_approval_context(
    request: &EffectRequest,
) -> Result<Option<CommandApprovalContext>, GatewayError> {
    let Some(intent) = &request.command_intent else {
        return Ok(None);
    };
    if request.action != "shell.run" {
        return Err(GatewayError::Safety(
            "command intent is only valid on shell.run".into(),
        ));
    }
    intent
        .validate()
        .map_err(|message| GatewayError::Safety(message.into()))?;
    // Do not accept an attacker-sized display independently of the effect ceiling.
    if serde_json::to_vec(request).map_err(|_| invalid())?.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let args = request
        .content
        .get("args")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(invalid)?;
    let cwd = request
        .content
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(invalid)?;
    let mut secrets = request
        .content
        .get("environment")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|env| env.iter())
        .filter(|(name, _)| SECRET_FIELDS.is_match(name))
        .filter_map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    secrets.sort_unstable();
    secrets.dedup();
    let mut redacted = false;
    let credential_profiles = credential_options::for_invocation(&request.resource, args);
    let mut project = |text: &str| {
        let (text, changed) = sanitized(text, &secrets, &credential_profiles);
        redacted |= changed;
        text
    };
    let justification = project(&intent.justification);
    let executable = project(&request.resource);
    let working_directory = project(cwd);
    let mut arguments = Vec::with_capacity(args.len());
    let mut secret_next = false;
    let mut argument_redacted = false;
    for argument in args {
        let argument = argument.as_str().ok_or_else(invalid)?;
        if secret_next {
            arguments.push(REDACTED.into());
            secret_next = false;
            argument_redacted = true;
        } else {
            let value_start = credential_option_value_start(argument, &credential_profiles);
            if let Some(start) = value_start.filter(|start| *start < argument.len()) {
                // An attached argv value extends to the argument boundary, not
                // to a shell delimiter within it (cookies may contain spaces/;).
                arguments.push(format!("{}{REDACTED}", project(&argument[..start])));
                argument_redacted = true;
            } else {
                secret_next = shell_value::is_dynamic(argument)
                    || (value_start.is_some() && !argument.contains('='));
                arguments.push(project(argument));
            }
        }
    }
    // An argument-level replacement may not have passed through `project`.
    redacted |= argument_redacted;
    let context = CommandApprovalContext {
        justification,
        executable,
        arguments,
        working_directory,
        redacted,
    };
    context.validate().map_err(|_| invalid())?;
    Ok(Some(context))
}

fn credential_option_value_start(
    argument: &str,
    profiles: &[&credential_options::Profile],
) -> Option<usize> {
    let (name, value) = argument
        .split_once('=')
        .map_or((argument, None), |(name, value)| (name, Some(value)));
    if (name.starts_with("--") && SECRET_FIELDS.is_match(name))
        || CREDENTIAL_VALUE_OPTIONS.contains(&name)
    {
        return Some(name.len() + usize::from(value.is_some()));
    }
    if let Some(start) = profiles
        .iter()
        .find_map(|profile| profile.value_start(argument))
    {
        return Some(start);
    }
    SHORT_CREDENTIAL_OPTION
        .find(argument)
        .map(|matched| matched.end())
}

fn invalid() -> GatewayError {
    GatewayError::Safety("command approval details cannot be displayed safely".into())
}

fn sanitized(
    text: &str,
    secrets: &[&str],
    profiles: &[&credential_options::Profile],
) -> (String, bool) {
    // Detect every credential against the original display input. Replacements
    // must not destroy a later recognizer's field name or token boundary.
    // One mask byte per input byte bounds memory independently of the number
    // of matching secrets. Union matches immediately instead of materializing
    // and sorting a potentially multiplicative vector of duplicate ranges.
    let mut mask = vec![false; text.len()];
    for matched in PRIVATE_KEY.find_iter(text) {
        mask[matched.range()].fill(true);
    }
    for (pattern, retained_suffix) in [(&*URL_USERINFO, 1), (&*CREDENTIAL_HEADERS, 0)] {
        for captures in pattern.captures_iter(text) {
            let range =
                captures.get(1).unwrap().end()..captures.get(0).unwrap().end() - retained_suffix;
            mask[range].fill(true);
        }
    }
    let (spelling, original_ends) = shell_value::literal_spelling(text);
    // A literal URL or PEM marker may itself be split by shell quoting. Match
    // that auxiliary spelling too, but mask only original display bytes.
    for captures in URL_USERINFO.captures_iter(&spelling) {
        let start = original_ends[captures.get(1).unwrap().end() - 1];
        let end = original_ends[captures.get(0).unwrap().end() - 1] - 1; // ASCII @
        mask[start..end].fill(true);
    }
    for matched in PRIVATE_KEY.find_iter(&spelling) {
        let start = original_ends[matched.start()] - 1; // ASCII opening dash
        let end = original_ends[matched.end() - 1];
        mask[start..end].fill(true);
    }
    for range in shell_value::ranges(
        text,
        &[&*AUTHORIZATION, &*SECRET_ASSIGNMENTS, &*CREDENTIAL_FLAGS],
        &spelling,
        &original_ends,
    ) {
        mask[range].fill(true);
    }
    for range in credential_options::ranges(text, &spelling, &original_ends, profiles) {
        mask[range].fill(true);
    }
    shell_value::mask_dynamic_words(text, &mut mask);
    // Known values must not hide expansion syntax from the dynamic-name pass.
    for secret in secrets {
        for (start, _) in text.match_indices(secret) {
            mask[start..start + secret.len()].fill(true);
        }
    }
    let redacted = mask.iter().any(|masked| *masked);
    let mut value = String::with_capacity(text.len());
    let mut was_masked = false;
    for (index, character) in text.char_indices() {
        if mask[index] {
            if !was_masked {
                value.push_str(REDACTED);
            }
        } else {
            value.push(character);
        }
        was_masked = mask[index];
    }
    let escaped = value
        .chars()
        .flat_map(|character| {
            if character == '\\' || unsafe_command_display_character(character) {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect();
    (escaped, redacted)
}

#[cfg(test)]
mod tests;
