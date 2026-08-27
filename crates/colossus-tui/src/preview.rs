use super::*;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use std::{
    collections::BTreeMap,
    io::Cursor,
    sync::mpsc::{Receiver, channel},
    thread,
};

const PREVIEW_CACHE_ENTRIES: usize = 8;
const PREVIEW_CACHE_BYTES: u64 = 64 * 1_048_576;
pub(super) const PREVIEW_VARIANTS_PER_ASSET: usize = 4;

pub(super) fn decode_preview(bytes: Vec<u8>) -> Result<DynamicImage, String> {
    let format = image::guess_format(&bytes).map_err(|_| "preview format is invalid".to_owned())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("preview format is unsupported".into());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(PREVIEW_CACHE_BYTES);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| "preview could not be decoded within the 64 MiB cache bound".into())
}

pub(super) struct PreviewCache {
    picker: Picker,
    native_graphics: bool,
    assets: BTreeMap<String, PreviewAsset>,
    tick: u64,
    memory_bytes: u64,
}

struct PreviewAsset {
    image: DynamicImage,
    decoded_bytes: u64,
    last_used: u64,
    variants: BTreeMap<(u16, u16), PreviewVariant>,
}

impl PreviewAsset {
    fn memory_bytes(&self) -> u64 {
        self.decoded_bytes.saturating_mul(
            u64::try_from(self.variants.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
    }
}

struct PreviewVariant {
    protocol: ThreadProtocol,
    responses: Receiver<ResizeResponse>,
    lines: Vec<Line<'static>>,
    last_used: u64,
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self {
            picker: Picker::halfblocks(),
            native_graphics: false,
            assets: BTreeMap::new(),
            tick: 0,
            memory_bytes: 0,
        }
    }
}

impl PreviewCache {
    pub(super) fn set_picker(&mut self, picker: Picker) {
        self.native_graphics = picker.protocol_type() != ProtocolType::Halfblocks;
        self.picker = picker;
        self.assets.clear();
        self.memory_bytes = 0;
    }

    pub(super) fn contains(&self, digest: &str) -> bool {
        self.assets.contains_key(digest)
    }

    pub(super) fn native_graphics(&self) -> bool {
        self.native_graphics
    }

    pub(super) fn insert(&mut self, digest: String, image: DynamicImage) {
        let decoded_bytes = u64::from(image.width())
            .saturating_mul(u64::from(image.height()))
            .saturating_mul(4);
        if decoded_bytes > PREVIEW_CACHE_BYTES {
            return;
        }
        if let Some(previous) = self.assets.remove(&digest) {
            self.memory_bytes = self.memory_bytes.saturating_sub(previous.memory_bytes());
        }
        while self.assets.len() >= PREVIEW_CACHE_ENTRIES
            || self.memory_bytes.saturating_add(decoded_bytes) > PREVIEW_CACHE_BYTES
        {
            let Some(oldest) = self
                .assets
                .iter()
                .min_by_key(|(_, asset)| asset.last_used)
                .map(|(digest, _)| digest.clone())
            else {
                return;
            };
            if let Some(removed) = self.assets.remove(&oldest) {
                self.memory_bytes = self.memory_bytes.saturating_sub(removed.memory_bytes());
            }
        }
        self.tick = self.tick.wrapping_add(1);
        self.memory_bytes = self.memory_bytes.saturating_add(decoded_bytes);
        self.assets.insert(
            digest,
            PreviewAsset {
                image,
                decoded_bytes,
                last_used: self.tick,
                variants: BTreeMap::new(),
            },
        );
    }

    pub(super) fn prepare(&mut self, digest: &str, size: Size) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        let key = (size.width, size.height);
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let Some(asset) = self.assets.get(digest) else {
            return;
        };
        if !asset.variants.contains_key(&key) {
            let decoded_bytes = asset.decoded_bytes;
            let image = asset.image.clone();
            let oldest_variant = if asset.variants.len() >= PREVIEW_VARIANTS_PER_ASSET {
                asset
                    .variants
                    .iter()
                    .min_by_key(|(_, variant)| variant.last_used)
                    .map(|(key, _)| *key)
            } else {
                None
            };
            if let Some(oldest_variant) = oldest_variant
                && self
                    .assets
                    .get_mut(digest)
                    .and_then(|asset| asset.variants.remove(&oldest_variant))
                    .is_some()
            {
                self.memory_bytes = self.memory_bytes.saturating_sub(decoded_bytes);
            }
            if self.memory_bytes.saturating_add(decoded_bytes) > PREVIEW_CACHE_BYTES {
                return;
            }
            let (request_tx, request_rx) = channel::<ResizeRequest>();
            let (response_tx, response_rx) = channel::<ResizeResponse>();
            let _ = thread::Builder::new()
                .name("colossus-image-preview".into())
                .spawn(move || {
                    while let Ok(request) = request_rx.recv() {
                        if let Ok(response) = request.resize_encode()
                            && response_tx.send(response).is_err()
                        {
                            break;
                        }
                    }
                });
            let variant = PreviewVariant {
                protocol: ThreadProtocol::new(
                    request_tx,
                    Some(self.picker.new_resize_protocol(image)),
                ),
                responses: response_rx,
                lines: Vec::new(),
                last_used: tick,
            };
            let Some(asset) = self.assets.get_mut(digest) else {
                return;
            };
            asset.variants.insert(key, variant);
            self.memory_bytes = self.memory_bytes.saturating_add(decoded_bytes);
        }
        let Some(asset) = self.assets.get_mut(digest) else {
            return;
        };
        asset.last_used = tick;
        let Some(variant) = asset.variants.get_mut(&key) else {
            return;
        };
        variant.last_used = tick;
        let mut updated = false;
        while let Ok(response) = variant.responses.try_recv() {
            updated |= variant.protocol.update_resized_protocol(response);
        }
        if updated || variant.lines.is_empty() {
            let area = Rect::new(0, 0, size.width, size.height);
            let mut buffer = Buffer::empty(area);
            StatefulWidget::render(
                StatefulImage::new().resize(Resize::Fit(None)),
                area,
                &mut buffer,
                &mut variant.protocol,
            );
            variant.lines = if self.native_graphics {
                (0..size.height)
                    .map(|_| Line::from(" ".repeat(usize::from(size.width))))
                    .collect()
            } else {
                buffer_lines(&buffer)
            };
        }
    }

    pub(super) fn render_native(
        &mut self,
        frame: &mut Frame<'_>,
        digest: &str,
        size: Size,
        area: Rect,
    ) {
        if !self.native_graphics || area.width != size.width || area.height != size.height {
            return;
        }
        let Some(variant) = self
            .assets
            .get_mut(digest)
            .and_then(|asset| asset.variants.get_mut(&(size.width, size.height)))
        else {
            return;
        };
        frame.render_stateful_widget(
            StatefulImage::new().resize(Resize::Fit(None)),
            area,
            &mut variant.protocol,
        );
    }

    pub(super) fn lines(&self, digest: &str, size: Size) -> Option<&[Line<'static>]> {
        self.assets
            .get(digest)
            .and_then(|asset| asset.variants.get(&(size.width, size.height)))
            .map(|variant| variant.lines.as_slice())
            .filter(|lines| !lines.is_empty())
    }

    #[cfg(test)]
    pub(super) fn variant_count(&self, digest: &str) -> usize {
        self.assets
            .get(digest)
            .map_or(0, |asset| asset.variants.len())
    }
}

fn buffer_lines(buffer: &Buffer) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(usize::from(buffer.area.height));
    for y in buffer.area.top()..buffer.area.bottom() {
        let spans = (buffer.area.left()..buffer.area.right())
            .map(|x| {
                let cell = &buffer[(x, y)];
                Span::styled(cell.symbol().to_owned(), cell.style())
            })
            .collect::<Vec<_>>();
        lines.push(Line::from(spans));
    }
    lines
}
