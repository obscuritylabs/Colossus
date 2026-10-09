//! Safe request headers, bounded bodies, and create-only WORM audit requests.

use super::*;

pub(super) fn build_request(
    client: &reqwest::Client,
    url: &Url,
    request: &EffectRequest,
    obligations: &PolicyObligations,
) -> Result<reqwest::RequestBuilder, ExecutionError> {
    let worm_write = request.action == "audit.export.worm.write";
    let method = if worm_write {
        if request.content.get("method").and_then(Value::as_str) != Some("PUT")
            || request.content.get("create_only").and_then(Value::as_bool) != Some(true)
        {
            return Err(adapter_failure(
                "WORM audit export requires an explicit create-only PUT",
            ));
        }
        reqwest::Method::PUT
    } else {
        request
            .content
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .parse()
            .map_err(adapter_failure)?
    };
    let mut builder = client.request(method, url.clone());
    if worm_write {
        if request.credential_references.len() > 1 {
            return Err(adapter_failure(
                "WORM audit export accepts at most one credential reference",
            ));
        }
        if let Some(reference) = request.credential_references.first() {
            let variable = reference.reference.strip_prefix("env:").ok_or_else(|| {
                adapter_failure("WORM audit credential must be environment-backed")
            })?;
            if obligations.resource_authority != ResourceAuthority::Ambient
                && !obligations
                    .allowed_environment
                    .iter()
                    .any(|allowed| allowed == variable)
            {
                return Err(adapter_failure(
                    "WORM audit credential is absent from permit obligations",
                ));
            }
            let secret = std::env::var(variable).map_err(|_| {
                adapter_failure(format!("environment variable {variable} is unset"))
            })?;
            if secret.is_empty() {
                return Err(adapter_failure("resolved WORM audit credential is empty"));
            }
            builder = builder.bearer_auth(secret);
        }
        let encoded = request
            .content
            .get("body_base64")
            .and_then(Value::as_str)
            .ok_or_else(|| adapter_failure("WORM audit export requires a body"))?;
        let body = BASE64.decode(encoded).map_err(adapter_failure)?;
        if u64::try_from(body.len()).map_err(adapter_failure)? > obligations.max_output_bytes {
            return Err(adapter_failure(
                "HTTP request body exceeds the permitted bound",
            ));
        }
        let content_hash = request
            .content
            .get("content_sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| adapter_failure("WORM audit export requires a content hash"))?;
        if content_hash.len() != 64
            || !content_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            || content_hash != sha256_hex(&body)
        {
            return Err(adapter_failure(
                "WORM audit export content hash does not match the body",
            ));
        }
        let expected_suffix = format!("-{content_hash}.json");
        if !url.path().ends_with(&expected_suffix) {
            return Err(adapter_failure(
                "WORM audit export object key is not bound to the content hash",
            ));
        }
        builder = builder
            .header("content-type", "application/json")
            .header("if-none-match", "*")
            .header("x-content-sha256", content_hash)
            .body(body);
    } else if let Some(headers) = request.content.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            let normalized = name.to_ascii_lowercase();
            if !matches!(
                normalized.as_str(),
                "accept" | "content-type" | "user-agent"
            ) {
                return Err(adapter_failure(format!(
                    "HTTP header {name} is not in the safe adapter allowlist"
                )));
            }
            let value = value
                .as_str()
                .ok_or_else(|| adapter_failure("HTTP header values must be strings"))?;
            builder = builder.header(name, value);
        }
    }
    if !worm_write && let Some(encoded) = request.content.get("body_base64").and_then(Value::as_str)
    {
        let body = BASE64.decode(encoded).map_err(adapter_failure)?;
        if u64::try_from(body.len()).map_err(adapter_failure)? > obligations.max_output_bytes {
            return Err(adapter_failure(
                "HTTP request body exceeds the permitted bound",
            ));
        }
        builder = builder.body(body);
    }
    Ok(builder)
}
