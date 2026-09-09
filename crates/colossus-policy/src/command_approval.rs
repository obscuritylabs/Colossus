//! One credential-free display projection; never changes executable input.

use std::sync::LazyLock;

use colossus_contracts::{CommandApprovalContext, EffectRequest, unsafe_command_display_character};
use regex::{Captures, Regex};

use crate::GatewayError;

const REDACTED: &str = "[REDACTED]";

static SECRET_FIELDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(password|passwd|secret|token|api[-_]?key|authorization|credential|private[-_]?key|cookie)",
    )
    .expect("constant credential field pattern")
});
static SECRET_ASSIGNMENTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)((?:[\w-]*(?:password|passwd|secret|token|api[-_]?key|authorization|credential|private[-_]?key|cookie)[\w-]*)[\s=:]+)(?:"(?:\\.|[^"\\])*"|'[^']*'|[^\s;&|"']+)"#)
        .expect("constant credential assignment pattern")
});
static AUTHORIZATION: LazyLock<Regex> = LazyLock::new(|| {
    // Include the complete token68 alphabet, including `~`, so no credential
    // suffix survives a partial match in the released approval context.
    Regex::new(r"(?i)(\b(?:bearer|basic)\s+)[a-z0-9~+/_.=:-]+")
        .expect("constant authorization pattern")
});
static CREDENTIAL_HEADERS: LazyLock<Regex> = LazyLock::new(|| {
    // Headers are often one quoted shell word or one argv element. Remove the
    // whole value, including all cookie pairs, rather than just its first token.
    Regex::new(r#"(?i)(\b(?:cookie|set-cookie|authorization|proxy-authorization|x-api-key)\s*:\s*)[^\r\n"']+"#)
        .expect("constant credential header pattern")
});
static URL_USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([a-z][a-z0-9+.-]*://)[^\s/@]+@").expect("constant URL credential pattern")
});
static CREDENTIAL_FLAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"((?:^|[\s;&|])(?:(?:-u|-U)[\s=]*|(?:--user|--proxy-user|--oauth2-bearer|--cookie)[\s=]+))(?:"(?:\\.|[^"\\])*"|'[^']*'|[^\s;&|"']+)"#)
        .expect("constant credential flag pattern")
});
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----")
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
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    let mut redacted = false;
    let mut project = |text: &str| {
        let (text, changed) = sanitized(text, &secrets);
        redacted |= changed;
        text
    };
    let justification = project(&intent.justification);
    let executable = project(&request.resource);
    let working_directory = project(cwd);
    let mut arguments = Vec::with_capacity(args.len());
    let mut secret_next = false;
    for argument in args {
        let argument = argument.as_str().ok_or_else(invalid)?;
        if secret_next {
            arguments.push(REDACTED.into());
            secret_next = false;
        } else {
            secret_next = argument.starts_with('-')
                && !argument.contains('=')
                && (SECRET_FIELDS.is_match(argument)
                    || matches!(
                        argument,
                        "-u" | "-U" | "--user" | "--proxy-user" | "--oauth2-bearer"
                    ));
            arguments.push(project(argument));
        }
    }
    // An argument-level replacement may not have passed through `project`.
    redacted |= arguments.iter().any(|argument| argument == REDACTED);
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

fn invalid() -> GatewayError {
    GatewayError::Safety("command approval details cannot be displayed safely".into())
}

fn sanitized(text: &str, secrets: &[&str]) -> (String, bool) {
    let mut value = text.to_owned();
    for secret in secrets {
        value = value.replace(secret, REDACTED);
    }
    value = PRIVATE_KEY.replace_all(&value, REDACTED).into_owned();
    for pattern in [
        &*URL_USERINFO,
        &*AUTHORIZATION,
        &*CREDENTIAL_HEADERS,
        &*SECRET_ASSIGNMENTS,
        &*CREDENTIAL_FLAGS,
    ] {
        value = pattern
            .replace_all(&value, |captures: &Captures<'_>| {
                format!(
                    "{}{REDACTED}{}",
                    &captures[1],
                    if std::ptr::eq(pattern, &*URL_USERINFO) {
                        "@"
                    } else {
                        ""
                    }
                )
            })
            .into_owned();
    }
    let redacted = value != text;
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
