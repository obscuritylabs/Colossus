//! Optional display assets from the Colossus client namespace. No network or SVG execution.

use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_contracts::{PluginInventoryEntry, PluginOrigin};
use image::{ImageFormat, ImageReader, Limits};
use std::io::Cursor;

const NAMESPACE: &str = "com.obscuritylabs.colossus";
const MAX_ICON_BYTES: u64 = 64 * 1024;
const MAX_ICON_DIMENSION: u32 = 512;
const MAX_INVENTORY_ICON_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn bound_inventory_icons(entries: &mut [PluginInventoryEntry]) {
    let mut bytes = 0_usize;
    // Keep the first-party identity even when many installed plugins consume the
    // display budget. Do not change catalog order or remove component metadata.
    for bundled in [true, false] {
        for entry in entries
            .iter_mut()
            .filter(|entry| (entry.origin == PluginOrigin::Bundled) == bundled)
        {
            if let Some(icon) = &entry.icon_data_url {
                if bytes.saturating_add(icon.len()) > MAX_INVENTORY_ICON_BYTES {
                    entry.icon_data_url = None;
                } else {
                    bytes += icon.len();
                }
            }
        }
    }
}

pub(super) fn load_icon(
    root: &Path,
    manifest: &AgentPluginManifest,
    diagnostics: &mut Vec<PluginComponentDiagnostic>,
) -> Option<String> {
    let extension = manifest.extensions.get(NAMESPACE)?;
    let result = extension
        .as_object()
        .ok_or_else(|| adapter("Colossus extension must be an object"))
        .and_then(|extension| {
            extension
                .get("icon")
                .map(|icon| read_icon(root, icon))
                .transpose()
        });
    match result {
        Ok(icon) => icon,
        Err(_) => {
            // Do not disclose supplied paths or image/manifest content in diagnostics.
            diagnostics.push(component_diagnostic(
                PluginComponentKind::Plugin,
                None,
                "invalid_plugin_icon",
                "Icon ignored: use a PNG in com.obscuritylabs.colossus/, at most 64 KiB and 512 × 512 pixels",
            ));
            None
        }
    }
}

fn read_icon(root: &Path, icon: &Value) -> Result<String, StoreError> {
    let path = icon
        .as_str()
        .ok_or_else(|| adapter("icon must be a path"))?;
    if path.len() > 512
        || !path.starts_with(&format!("{NAMESPACE}/"))
        || !path.ends_with(".png")
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(adapter("invalid icon path"));
    }
    let bytes = read_contained(root, Path::new(path), MAX_ICON_BYTES)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_ICON_DIMENSION);
    limits.max_image_height = Some(MAX_ICON_DIMENSION);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(adapter)?;
    // Re-encode pixels so metadata, animation and trailing content never reach discovery.
    let mut output = Cursor::new(Vec::new());
    decoded
        .write_to(&mut output, ImageFormat::Png)
        .map_err(adapter)?;
    let output = output.into_inner();
    if output.len() as u64 > MAX_ICON_BYTES {
        return Err(adapter("normalized icon exceeds display limit"));
    }
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(output)))
}

#[cfg(test)]
#[path = "icons_tests.rs"]
mod tests;
