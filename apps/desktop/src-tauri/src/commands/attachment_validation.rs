//! Native attachment admission. The renderer supplies preferences, never format authority.

use crate::dto::CommandErrorDto;
use colossus_sdk::ConfiguredModelProfile;
use std::path::Path;

pub(super) fn require_document_support(
    profiles: &[ConfiguredModelProfile],
    role: &str,
    model_profile: Option<&str>,
    media_type: &str,
) -> Result<(), CommandErrorDto> {
    let selected = if let Some(profile) = model_profile {
        profiles.iter().find(|entry| entry.profile == profile)
    } else if role.trim().is_empty() {
        None
    } else {
        profiles
            .iter()
            .find(|entry| entry.default_roles.iter().any(|value| value == role))
            .or_else(|| {
                profiles
                    .iter()
                    .find(|entry| entry.default_roles.iter().any(|value| value == "primary"))
            })
    };
    if colossus_media::supported_document_media_type(media_type)
        && selected.is_some_and(|entry| {
            entry
                .document_media_types
                .iter()
                .any(|value| value == media_type)
        })
    {
        return Ok(());
    }
    Err(CommandErrorDto::invalid(
        "attachment",
        "This model does not advertise support for this document format. Enable Document inputs in model settings or choose a compatible model.",
    ))
}

pub(super) fn validate_attachment_bytes(
    file_name: &str,
    media_type: &str,
    bytes: &[u8],
) -> Result<(), CommandErrorDto> {
    if colossus_media::supported_document_media_type(media_type) {
        colossus_media::validate_document_bytes(file_name, Some(media_type), bytes).map_err(
            |_| {
                CommandErrorDto::invalid(
                    "attachment",
                    "The document does not match its format or exceeds the document limits.",
                )
            },
        )?;
    } else if media_type.starts_with("image/") {
        colossus_media::validate_image_bytes(file_name, Some(media_type), bytes).map_err(|_| {
            CommandErrorDto::invalid(
                "attachment",
                "Images must be valid static PNG, JPEG, or WebP files within the image limits.",
            )
        })?;
    } else if std::str::from_utf8(bytes).is_err() {
        return Err(CommandErrorDto::invalid(
            "attachment",
            "Text attachments must contain bounded UTF-8 text.",
        ));
    }
    Ok(())
}

pub(super) fn attachment_media_type(path: &Path) -> Option<&'static str> {
    if let Some(media_type) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(colossus_media::document_media_type_for_file_name)
    {
        return Some(media_type);
    }
    if matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("Dockerfile" | "Makefile" | ".gitignore")
    ) {
        return Some("text/plain");
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => Some("application/json"),
        Some("yaml" | "yml") => Some("application/yaml"),
        Some("toml") => Some("application/toml"),
        Some("xml") => Some("application/xml"),
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("csv") => Some("text/csv"),
        Some("tsv") => Some("text/tab-separated-values"),
        Some(
            "txt" | "md" | "rst" | "log" | "ini" | "conf" | "cfg" | "properties" | "diff" | "patch"
            | "ps1" | "psm1" | "psd1" | "bat" | "cmd" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py"
            | "go" | "java" | "c" | "h" | "cpp" | "hpp" | "cs" | "rb" | "php" | "sh" | "zsh"
            | "fish" | "css" | "scss" | "html" | "sql" | "graphql" | "proto",
        ) => Some("text/plain"),
        _ => None,
    }
}

#[cfg(test)]
#[path = "attachment_validation_tests.rs"]
mod tests;
