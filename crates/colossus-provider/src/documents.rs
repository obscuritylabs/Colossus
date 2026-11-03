//! Transient native document resolution under the existing generation permit.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use sha2::{Digest as _, Sha256};

const MAX_DOCUMENT_COUNT: usize = 4;
const MAX_DOCUMENT_BYTES: u64 = 16 * 1_048_576;
const MAX_DOCUMENT_TOTAL_BYTES: u64 = 32 * 1_048_576;

/// A provider can echo any prefix of inline bytes in an ordinary error string.
/// Document diagnostics therefore retain only fixed categorical evidence.
pub(super) fn redacted_document_error(value: Option<&Value>) -> Value {
    let code = value
        .and_then(|value| {
            value
                .pointer("/error/code")
                .or_else(|| value.pointer("/response/error/code"))
        })
        .and_then(Value::as_str)
        .filter(|code| {
            matches!(
                *code,
                "unsupported_file"
                    | "file_inputs_not_supported"
                    | "unsupported_document"
                    | "document_inputs_not_supported"
            )
        })
        .unwrap_or("document_request_rejected");
    json!({"type":"error", "error":{"code":code,
        "message":"Provider response content omitted for a document request."}})
}

/// Preserve exact pre-generation evidence before omitting all free-form content.
pub(super) fn document_rejection(
    value: &Value,
    status: u16,
) -> Option<crate::ProviderFeatureRejection> {
    (matches!(status, 400 | 422)
        && matches!(
            value.pointer("/error/code").and_then(Value::as_str),
            Some(
                "unsupported_file"
                    | "file_inputs_not_supported"
                    | "unsupported_document"
                    | "document_inputs_not_supported"
            )
        ))
    .then_some(crate::ProviderFeatureRejection {
        provider_feature_rejected: ProviderFeature::Documents,
        status,
    })
}

pub(super) fn is_document_data_url(value: &str) -> bool {
    value
        .strip_prefix("data:")
        .and_then(|value| value.split_once(";base64,"))
        .is_some_and(|(mime, _)| {
            ProviderKind::OpenAiResponses
                .document_media_types()
                .contains(&mime)
        })
}

struct NativeDocumentSource<'a> {
    field: Option<&'static str>,
    object: &'a Map<String, Value>,
    private_fields: &'static [&'static str],
}

fn native_document_source(value: &Value) -> Option<NativeDocumentSource<'_>> {
    let object = value.as_object()?;
    let (field, private_fields): (_, &'static [&'static str]) =
        match object.get("type")?.as_str()? {
            "input_file" => (None, &["file_data", "file_id", "file_url"]),
            "file" if object.get("file").is_some_and(Value::is_object) => {
                (Some("file"), &["file_data", "file_id", "file_url"])
            }
            "document" if object.get("source").is_some_and(Value::is_object) => {
                (Some("source"), &["data", "file_id", "url"])
            }
            "base64"
                if object
                    .get("media_type")
                    .and_then(Value::as_str)
                    .is_some_and(|mime| {
                        ProviderKind::OpenAiResponses
                            .document_media_types()
                            .contains(&mime)
                    }) =>
            {
                (None, &["data"])
            }
            _ => return None,
        };
    let object = match field {
        Some(field) => object.get(field)?.as_object()?,
        None => object,
    };
    private_fields
        .iter()
        .any(|field| object.get(*field).is_some_and(Value::is_string))
        .then_some(NativeDocumentSource {
            field,
            object,
            private_fields,
        })
}

pub(super) fn tool_metadata_field(object: &Map<String, Value>, key: &str) -> bool {
    matches!(key, "arguments" | "input_schema" | "parameters")
        || (key == "input" && object.get("type").and_then(Value::as_str) == Some("tool_use"))
}

/// Inline bodies and remote native-file handles never become reusable state.
/// Ordinary tool arguments and schema properties named file_id remain ordinary data.
pub(super) fn contains_private_document_data(value: &Value) -> bool {
    match value {
        Value::String(value) => is_document_data_url(value),
        Value::Array(values) => values.iter().any(contains_private_document_data),
        Value::Object(object) => {
            native_document_source(value).is_some()
                || object
                    .iter()
                    .filter(|(key, _)| !tool_metadata_field(object, key))
                    .any(|(_, value)| contains_private_document_data(value))
        }
        _ => false,
    }
}

pub(super) fn redacted_native_document(value: &Value) -> Option<Value> {
    let source = native_document_source(value)?;
    let redact_fields = |object: &Map<String, Value>| {
        Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let value =
                        if source.private_fields.contains(&key.as_str()) && value.is_string() {
                            Value::String("[REDACTED_DOCUMENT_SOURCE]".into())
                        } else {
                            crate::executor::redacted_image_payload(value)
                        };
                    (key.clone(), value)
                })
                .collect(),
        )
    };
    match source.field {
        None => Some(redact_fields(source.object)),
        Some(field) => Some(Value::Object(
            value
                .as_object()?
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        if key == field {
                            redact_fields(source.object)
                        } else {
                            crate::executor::redacted_image_payload(value)
                        },
                    )
                })
                .collect(),
        )),
    }
}

pub(super) fn validate_document_request(
    kind: ProviderKind,
    request: &ModelRequest,
) -> Result<(), ProviderError> {
    let mut count = 0_usize;
    let mut bytes = 0_u64;
    let mut identities = BTreeMap::new();
    for message in &request.messages {
        for file in message.content.files() {
            if message.role != ModelMessageRole::User
                || !kind
                    .document_media_types()
                    .contains(&file.media_type.as_str())
            {
                return Err(ProviderError::Configuration(
                    "document format is unsupported by the selected provider API or message role"
                        .into(),
                ));
            }
            count = count.saturating_add(1);
            bytes = bytes.checked_add(file.size_bytes).ok_or_else(|| {
                ProviderError::Configuration("document input size overflowed".into())
            })?;
            if count > MAX_DOCUMENT_COUNT
                || file.size_bytes == 0
                || file.size_bytes > MAX_DOCUMENT_BYTES
                || bytes > MAX_DOCUMENT_TOTAL_BYTES
            {
                return Err(ProviderError::Configuration(
                    "document inputs exceed the 4-file, 16 MiB-per-file or 32 MiB-combined bound"
                        .into(),
                ));
            }
            if let Some(existing) = identities.insert(&file.artifact_id, file)
                && existing != file
            {
                return Err(ProviderError::Configuration(
                    "one document artifact ID has conflicting verified metadata".into(),
                ));
            }
        }
    }
    Ok(())
}

impl ProviderExecutor {
    pub(super) async fn resolve_files(
        &self,
        request: &ModelRequest,
        resolved: &mut ProviderResolvedImages,
    ) -> Result<(), ProviderError> {
        // Check the complete format matrix and aggregate bounds before resolving any bytes.
        validate_document_request(self.profile.kind, request)?;
        let references = request
            .messages
            .iter()
            .flat_map(|message| message.content.files())
            .collect::<Vec<_>>();
        if references.is_empty() {
            return Ok(());
        }
        let resolver = self.run_input_media().ok_or_else(|| {
            ProviderError::Configuration("run-input document resolver is unavailable".into())
        })?;
        let mut seen = BTreeSet::new();
        for reference in references {
            if !seen.insert(&reference.artifact_id) {
                continue;
            }
            let file = resolver
                .resolve_file(reference)
                .await
                .map_err(|error| ProviderError::Configuration(error.to_string()))?;
            if &file.reference != reference
                || u64::try_from(file.bytes.len()).unwrap_or(u64::MAX) != reference.size_bytes
                || hex::encode(Sha256::digest(&file.bytes)) != reference.sha256
            {
                return Err(ProviderError::Configuration(
                    "resolved document does not match its exact verified reference".into(),
                ));
            }
            let data_url = format!(
                "data:{};base64,{}",
                reference.media_type,
                BASE64.encode(&file.bytes)
            );
            resolved.insert_file(reference, data_url)?;
        }
        Ok(())
    }
}
