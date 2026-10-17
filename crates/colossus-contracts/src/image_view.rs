use super::*;

/// Pending tool-turn capacity; older images follow normal context compaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageInputBudget {
    /// Number of additional images this pending tool turn can return.
    pub remaining_count: usize,
    /// Combined additional exact bytes this pending tool turn can return.
    pub remaining_bytes: u64,
}

/// Metadata returned only after `image.view` has read and released exact image bytes.
/// Bytes live in immutable artifacts, never in tool output or activity metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageViewOutput {
    /// Images in requested order.
    pub images: Vec<ModelImageReference>,
}

impl ImageViewOutput {
    /// Decode the successful built-in result, rejecting other tools and invalid references.
    pub fn from_tool_result(result: &ToolResult) -> Option<Self> {
        if result.name != "image.view" || result.exit_code != 0 || result.output.len() > 32_768 {
            return None;
        }
        let output: Self = serde_json::from_str(&result.output).ok()?;
        if output.images.is_empty() || output.images.len() > 16 {
            return None;
        }
        let mut combined = 0_u64;
        for image in &output.images {
            let digest = image.artifact_id.strip_prefix("artifact-")?;
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || image.file_name.is_empty()
                || image.file_name.len() > 255
                || image
                    .file_name
                    .chars()
                    .any(|character| character.is_control() || matches!(character, '/' | '\\'))
                || image.size_bytes == 0
                || image.size_bytes > 16 * 1_048_576
                || image.width_pixels == 0
                || image.height_pixels == 0
                || image.width_pixels > 16_384
                || image.height_pixels > 16_384
                || u64::from(image.width_pixels) * u64::from(image.height_pixels) > 100_000_000
                || image.sha256.len() != 64
                || !image
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || !matches!(
                    image.media_type.as_str(),
                    "image/png" | "image/jpeg" | "image/webp"
                )
            {
                return None;
            }
            combined = combined.checked_add(image.size_bytes)?;
            if combined > 32 * 1_048_576 {
                return None;
            }
        }
        Some(output)
    }
}
