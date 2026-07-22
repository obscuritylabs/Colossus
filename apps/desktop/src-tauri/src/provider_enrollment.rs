//! Native provider enrollment that keeps API keys outside `WebView` memory and IPC.

#[cfg(target_os = "macos")]
use std::{process::Stdio, time::Duration};

#[cfg(target_os = "macos")]
use tokio::io::AsyncReadExt as _;
use zeroize::Zeroizing;

use crate::{desktop_settings::ProviderKindSetting, dto::CommandErrorDto};

const MAX_PROVIDER_SECRET_BYTES: usize = 761;
#[cfg(target_os = "macos")]
const PROVIDER_PROMPT_OUTPUT_BYTES: u64 = (MAX_PROVIDER_SECRET_BYTES + 2) as u64;
#[cfg(target_os = "macos")]
const PROVIDER_PROMPT_TIMEOUT: Duration = Duration::from_mins(2);
#[cfg(target_os = "macos")]
const PROVIDER_REAP_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(target_os = "macos")]
pub(crate) async fn request_provider_secret(
    kind: ProviderKindSetting,
) -> Result<Zeroizing<String>, CommandErrorDto> {
    let script = provider_prompt_script(kind);
    let mut command = tokio::process::Command::new("/usr/bin/osascript");
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .args(["-e", script]);
    let mut child = command.spawn().map_err(|_| enrollment_error(true))?;
    let output = child.stdout.take().ok_or_else(|| enrollment_error(false))?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_PROVIDER_SECRET_BYTES + 2));
    let exchange = tokio::time::timeout(PROVIDER_PROMPT_TIMEOUT, async {
        output
            .take(PROVIDER_PROMPT_OUTPUT_BYTES)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| enrollment_error(true))?;
        if bytes.len() > MAX_PROVIDER_SECRET_BYTES + 1 {
            return Err(enrollment_error(false));
        }
        child.wait().await.map_err(|_| enrollment_error(true))
    })
    .await;
    let status = match exchange {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            kill_and_reap(&mut child).await;
            return Err(error);
        }
        Err(_) => {
            kill_and_reap(&mut child).await;
            return Err(enrollment_error(true));
        }
    };
    if !status.success() {
        return Err(enrollment_error(true));
    }

    // The fixed AppleScript rejects an answer above the native credential bound before
    // returning it. Wrap the captured pipe bytes immediately so every native copy is
    // zeroized without ever formatting the value into an error.
    let bytes = bytes
        .strip_suffix(b"\n")
        .ok_or_else(|| enrollment_error(false))?;
    let secret = std::str::from_utf8(bytes).map_err(|_| enrollment_error(false))?;
    validate_secret(kind, secret)?;
    Ok(Zeroizing::new(secret.to_owned()))
}

#[cfg(target_os = "macos")]
async fn kill_and_reap(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    let _ = tokio::time::timeout(PROVIDER_REAP_TIMEOUT, child.wait()).await;
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn request_provider_secret(
    _kind: ProviderKindSetting,
) -> Result<Zeroizing<String>, CommandErrorDto> {
    Err(CommandErrorDto::local_sanitized(
        "provider_enrollment_unsupported",
        "Native provider enrollment is unavailable on this platform.",
        false,
    ))
}

#[cfg(target_os = "macos")]
fn provider_prompt_script(kind: ProviderKindSetting) -> &'static str {
    match kind {
        ProviderKindSetting::OpenAiResponses => {
            r#"set providerResponse to display dialog "Enter the OpenAI API key for api.openai.com. Colossus will store it in your login keychain." default answer "" with title "Configure OpenAI for Colossus" buttons {"Cancel", "Save"} default button "Save" cancel button "Cancel" with hidden answer
set providerKey to text returned of providerResponse
if (length of providerKey) < 1 or (length of providerKey) > 761 then error number 64
return providerKey"#
        }
        ProviderKindSetting::OpenAiCompatible => {
            r#"set providerResponse to display dialog "Enter the OpenRouter API key for openrouter.ai. Colossus will store it in your login keychain." default answer "" with title "Configure OpenRouter for Colossus" buttons {"Cancel", "Save"} default button "Save" cancel button "Cancel" with hidden answer
set providerKey to text returned of providerResponse
if (length of providerKey) < 1 or (length of providerKey) > 761 then error number 64
return providerKey"#
        }
    }
}

fn validate_secret(kind: ProviderKindSetting, secret: &str) -> Result<(), CommandErrorDto> {
    let expected_prefix = match kind {
        ProviderKindSetting::OpenAiResponses => "sk-",
        ProviderKindSetting::OpenAiCompatible => "sk-or-v1-",
    };
    if secret.is_empty()
        || secret.len() > MAX_PROVIDER_SECRET_BYTES
        || !secret.starts_with(expected_prefix)
        || secret.len() <= expected_prefix.len()
        || !secret.bytes().all(|byte| (b'!'..=b'~').contains(&byte))
    {
        return Err(CommandErrorDto::invalid(
            "providerCredential",
            "The provider key is empty or has an invalid format.",
        ));
    }
    Ok(())
}

fn enrollment_error(retryable: bool) -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "provider_enrollment",
        "The native provider key prompt was cancelled or could not finish.",
        retryable,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_settings::{OPENAI_BASE_URL, OPENROUTER_BASE_URL, provider_base_url};

    #[test]
    fn provider_origins_are_fixed_by_native_kind() {
        assert_eq!(
            provider_base_url(ProviderKindSetting::OpenAiResponses),
            OPENAI_BASE_URL
        );
        assert_eq!(
            provider_base_url(ProviderKindSetting::OpenAiCompatible),
            OPENROUTER_BASE_URL
        );
    }

    #[test]
    fn provider_secret_validation_is_bounded_and_whitespace_strict() {
        assert!(validate_secret(ProviderKindSetting::OpenAiResponses, "sk-example").is_ok());
        assert!(validate_secret(ProviderKindSetting::OpenAiCompatible, "sk-or-v1-example").is_ok());
        for secret in ["", " sk-example", "sk-example ", "sk-\nexample"] {
            assert!(
                validate_secret(ProviderKindSetting::OpenAiResponses, secret).is_err(),
                "accepted {secret:?}"
            );
        }
        assert!(validate_secret(ProviderKindSetting::OpenAiCompatible, "sk-example").is_err());
        assert!(
            validate_secret(
                ProviderKindSetting::OpenAiResponses,
                &format!("sk-{}", "x".repeat(MAX_PROVIDER_SECRET_BYTES))
            )
            .is_err()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_prompts_name_only_the_fixed_provider_origin() {
        let openai = provider_prompt_script(ProviderKindSetting::OpenAiResponses);
        let openrouter = provider_prompt_script(ProviderKindSetting::OpenAiCompatible);
        assert!(openai.contains("api.openai.com"));
        assert!(!openai.contains("openrouter.ai"));
        assert!(openrouter.contains("openrouter.ai"));
        assert!(!openrouter.contains("api.openai.com"));
        for script in [openai, openrouter] {
            assert!(script.contains("with hidden answer"));
            assert!(script.contains("(length of providerKey) > 761"));
        }
    }
}
