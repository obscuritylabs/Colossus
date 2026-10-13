//! Native marks retain their original writer/document/geometry/deadline.
use super::*;
use colossus_browser_presentation::Lease;
use colossus_contracts::{BrowserAnnotation, BrowserAnnotationColor, BrowserAnnotationTarget};

#[derive(Clone)]
pub(super) struct Pending {
    tab: u64,
    document: u64,
    control: u64,
    geometry: (u32, u32, u32),
    marks: Vec<ffi::Annotation>,
    expires: Instant,
}
impl Host {
    pub(super) fn clear_annotations(&mut self, native: u64) {
        self.presentation.annotations = None;
        // SAFETY: main-thread owned tab; native code independently checks its document.
        let _ = unsafe {
            ffi::colossus_cef_presentation_annotations(
                native,
                1,
                self.native_document(native).unwrap_or(0),
                std::ptr::null(),
                0,
                0,
            )
        };
    }
    pub(super) fn annotate(
        &mut self,
        native: u64,
        identity: &BrowserTabId,
        annotations: &[BrowserAnnotation],
        ttl_ms: u32,
        scale: f64,
        control: &BrowserDriverControl,
    ) -> Result<(), BrowserDriverError> {
        if !(1..=16).contains(&annotations.len()) || ttl_ms == 0 || ttl_ms > 300_000 {
            return Err(BrowserDriverError::Denied);
        }
        let mut marks = Vec::with_capacity(annotations.len());
        for annotation in annotations {
            if annotation.label.len() > 160 || annotation.label.chars().any(char::is_control) {
                return Err(BrowserDriverError::Denied);
            }
            let (left, top, width, height) = match &annotation.target {
                BrowserAnnotationTarget::Rectangle { rect } => (
                    f64::from(rect.x) / scale,
                    f64::from(rect.y) / scale,
                    f64::from(rect.width) / scale,
                    f64::from(rect.height) / scale,
                ),
                BrowserAnnotationTarget::Element { element } => {
                    let (model, _) =
                        self.pointer_target(native, identity, element, false, control)?;
                    let points = model
                        .pointer("/model/border")
                        .or_else(|| model.pointer("/model/content"))
                        .and_then(Value::as_array)
                        .ok_or(BrowserDriverError::Stale)?;
                    if points.len() != 8 {
                        return Err(BrowserDriverError::Stale);
                    }
                    let mut xs = Vec::new();
                    let mut ys = Vec::new();
                    for point in points.chunks_exact(2) {
                        xs.push(point[0].as_f64().ok_or(BrowserDriverError::Stale)?);
                        ys.push(point[1].as_f64().ok_or(BrowserDriverError::Stale)?);
                    }
                    let left = xs.iter().copied().fold(f64::INFINITY, f64::min);
                    let right = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let top = ys.iter().copied().fold(f64::INFINITY, f64::min);
                    let bottom = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    (left, top, right - left, bottom - top)
                }
            };
            let (w, h) = self.viewport_dimensions();
            if ![left, top, width, height].iter().all(|v| v.is_finite())
                || left < 0.0
                || top < 0.0
                || width <= 0.0
                || height <= 0.0
                || left + width > f64::from(w)
                || top + height > f64::from(h)
            {
                return Err(BrowserDriverError::Stale);
            }
            let mut label = [0u16; 160];
            let units: Vec<_> = annotation.label.encode_utf16().collect();
            label[..units.len()].copy_from_slice(&units);
            let color = match annotation.color {
                BrowserAnnotationColor::Blue => 0,
                BrowserAnnotationColor::Red => 1,
                BrowserAnnotationColor::Green => 2,
                BrowserAnnotationColor::Yellow => 3,
            };
            marks.push(ffi::Annotation {
                left,
                top,
                width,
                height,
                color,
                label_units: units.len() as u32,
                label,
            });
        }
        self.check_document_fence()?;
        self.presentation.annotations = Some(Pending {
            tab: native,
            document: self.native_document(native)?,
            control: self.presentation.control_generation,
            geometry: self.presentation.geometry(),
            marks,
            expires: Instant::now() + Duration::from_millis(u64::from(ttl_ms)),
        });
        self.paint_annotations(native);
        Ok(())
    }
    fn paint_annotations(&self, native: u64) {
        let Some(pending) = self.presentation.annotations.as_ref().filter(|p| {
            p.valid(
                native,
                self.native_document(native).unwrap_or(0),
                self.presentation.control_generation,
                self.presentation.geometry(),
            )
        }) else {
            return;
        };
        let ttl = pending
            .expires
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(300_000) as u32;
        // SAFETY: synchronous CEF UI thread, bounded owned marks copied by native code.
        let _ = unsafe {
            ffi::colossus_cef_presentation_annotations(
                native,
                1,
                pending.document,
                pending.marks.as_ptr(),
                pending.marks.len(),
                ttl,
            )
        };
    }
    pub(super) fn replay_annotations(&mut self, lease: Lease) {
        if self.presentation.annotations.as_ref().is_some_and(|p| {
            !p.valid(
                lease.tab,
                lease.document_generation,
                lease.control_generation,
                self.presentation.geometry(),
            )
        }) {
            self.presentation.annotations = None;
        }
        self.paint_annotations(lease.tab);
    }
    pub(super) fn annotated_png(
        &self,
        native: u64,
        bytes: &[u8],
        max_bytes: u32,
    ) -> Result<Vec<u8>, BrowserDriverError> {
        let pending = self
            .presentation
            .annotations
            .as_ref()
            .filter(|p| {
                p.valid(
                    native,
                    self.native_document(native).unwrap_or(0),
                    self.presentation.control_generation,
                    self.presentation.geometry(),
                )
            })
            .ok_or(BrowserDriverError::Stale)?;
        let mut output = zeroize::Zeroizing::new(vec![0; max_bytes as usize]);
        let mut written = 0;
        // SAFETY: bounded PNG bytes and UTF-16 marks, output has exactly capacity.
        status_result(unsafe {
            ffi::colossus_cef_annotation_png(
                bytes.as_ptr(),
                bytes.len(),
                pending.marks.as_ptr(),
                pending.marks.len(),
                f64::from(pending.geometry.2) / 1000.0,
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        })?;
        if written > output.len() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        output.truncate(written);
        Ok(std::mem::take(&mut *output))
    }
}
impl Pending {
    fn valid(&self, tab: u64, document: u64, control: u64, geometry: (u32, u32, u32)) -> bool {
        self.tab == tab
            && self.document == document
            && self.control == control
            && control != 0
            && self.geometry == geometry
            && Instant::now() < self.expires
    }
}
