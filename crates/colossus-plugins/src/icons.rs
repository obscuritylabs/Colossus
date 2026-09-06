//! Optional display assets from the Colossus client namespace. No network or SVG execution.

use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_contracts::PluginOrigin;
use image::{ImageFormat, ImageReader, Limits};
use std::io::Cursor;

const NAMESPACE: &str = "com.obscuritylabs.colossus";
const MAX_ICON_BYTES: u64 = 64 * 1024;
const MAX_ICON_DIMENSION: u32 = 512;
const MAX_INVENTORY_ICON_BYTES: usize = 2 * 1024 * 1024;

const MAX_ENCODED_ICON_BYTES: usize = 21 + (MAX_ICON_BYTES as usize).div_ceil(3) * 4;

pub(crate) struct IconBudget {
    bytes: usize,
    pixels: u32,
    images: u32,
}

impl Default for IconBudget {
    fn default() -> Self {
        Self {
            bytes: MAX_INVENTORY_ICON_BYTES,
            pixels: 8 * 1024 * 1024,
            images: 64,
        }
    }
}

impl IconBudget {
    pub(crate) fn exhausted() -> Self {
        Self {
            bytes: 0,
            pixels: 0,
            images: 0,
        }
    }

    fn can_load(&self) -> bool {
        self.bytes > 0 && self.pixels > 0 && self.images > 0
    }
}

/// Reserve one maximum-size image for the executable-owned plugin without
/// changing catalog order or letting operator-installed names claim its budget.
pub(crate) struct CatalogIconBudget {
    bundled: IconBudget,
    installed: IconBudget,
}

impl Default for CatalogIconBudget {
    fn default() -> Self {
        let mut installed = IconBudget::default();
        installed.bytes -= MAX_ENCODED_ICON_BYTES;
        installed.pixels -= MAX_ICON_DIMENSION * MAX_ICON_DIMENSION;
        installed.images -= 1;
        Self {
            bundled: IconBudget {
                bytes: MAX_ENCODED_ICON_BYTES,
                pixels: MAX_ICON_DIMENSION * MAX_ICON_DIMENSION,
                images: 1,
            },
            installed,
        }
    }
}

impl CatalogIconBudget {
    pub(crate) fn for_origin(&mut self, origin: PluginOrigin) -> &mut IconBudget {
        if origin == PluginOrigin::Bundled {
            &mut self.bundled
        } else {
            &mut self.installed
        }
    }
}

pub(super) fn load_icon(
    root: &Path,
    manifest: &AgentPluginManifest,
    diagnostics: &mut Vec<PluginComponentDiagnostic>,
    budget: &mut IconBudget,
) -> Option<String> {
    if !budget.can_load() {
        return None;
    }
    let extension = manifest.extensions.get(NAMESPACE)?;
    let result = extension
        .as_object()
        .ok_or_else(|| adapter("Colossus extension must be an object"))
        .and_then(|extension| {
            extension
                .get("icon")
                .map(|icon| read_icon(root, icon, budget))
                .transpose()
        });
    match result {
        Ok(icon) => icon.flatten(),
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

fn read_icon(
    root: &Path,
    icon: &Value,
    budget: &mut IconBudget,
) -> Result<Option<String>, StoreError> {
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
    // Inspect IHDR before decoder construction, which can inflate ancillary data.
    if bytes.len() < 33 || &bytes[..16] != b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR" {
        return Err(adapter("invalid PNG header"));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(adapter)?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(adapter)?);
    if !(1..=MAX_ICON_DIMENSION).contains(&width) || !(1..=MAX_ICON_DIMENSION).contains(&height) {
        return Err(adapter("invalid icon dimensions"));
    }
    let pixels = width * height;
    if pixels > budget.pixels {
        return Ok(None);
    }
    budget.pixels -= pixels;
    budget.images -= 1;
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
    let url = format!("data:image/png;base64,{}", STANDARD.encode(output));
    if url.len() > budget.bytes {
        budget.bytes = 0;
        return Ok(None);
    }
    budget.bytes -= url.len();
    Ok(Some(url))
}

#[cfg(test)]
#[path = "icons_tests.rs"]
mod tests;
