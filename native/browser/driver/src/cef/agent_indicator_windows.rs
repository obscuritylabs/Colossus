//! Best-effort native decorations for already admitted Core input effects.
//! No shared protocol, page injection, screenshot mutation or OS cursor movement.
use super::{BrowserDriverControl, Host, Value, ffi, json};
use colossus_browser_presentation::Lease;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) struct Pending {
    tab: u64,
    document: u64,
    control_generation: u64,
    geometry: (u32, u32, u32),
    indicator: ffi::AgentIndicator,
    expires: Instant,
}

impl Host {
    pub(super) fn clear_agent_indicator(&mut self, native: u64) {
        self.presentation.agent_indicator = None;
        self.send_agent_indicator(native, &ffi::AgentIndicator::default());
    }

    pub(super) fn indicate_agent_focus(
        &mut self,
        native: u64,
        backend: i32,
        control: &BrowserDriverControl,
    ) {
        if let Ok(model) = self.method(
            native,
            "DOM.getBoxModel",
            json!({"backendNodeId":backend}),
            control,
        ) {
            self.indicate_agent_target(native, &model, 2);
        } else {
            self.clear_agent_indicator(native);
        }
    }

    pub(super) fn indicate_agent_target(&mut self, native: u64, model: &Value, kind: u32) {
        let Some(indicator) = geometry(model, self.viewport_dimensions(), kind) else {
            self.clear_agent_indicator(native);
            return;
        };
        self.send_agent_indicator(native, &indicator);
    }

    fn send_agent_indicator(&mut self, native: u64, indicator: &ffi::AgentIndicator) {
        if !self.presentation.enabled || self.presentation.human {
            return;
        }
        let Ok(document) = self.native_document(native) else {
            return;
        };
        // A navigating click must never paint its old coordinates on the new document.
        if indicator.kind != 0
            && self
                .expected_revision
                .is_none_or(|(tab, _, old)| tab != native || old != document)
        {
            return;
        }
        if indicator.kind != 0 {
            self.presentation.agent_indicator = Some(Pending {
                tab: native,
                document,
                control_generation: self.presentation.control_generation,
                geometry: self.presentation.geometry(),
                indicator: *indicator,
                expires: Instant::now() + Duration::from_millis(2200),
            });
        }
        paint(native, document, indicator);
    }

    // Read-only frame recovery can replace the lease during an admitted input
    // operation. Replay only the same document and geometry, with the original
    // remaining lifetime; renewing a viewer never extends a cursor's lifetime.
    pub(super) fn replay_agent_indicator(&mut self, lease: Lease) {
        let Some(pending) = self.presentation.agent_indicator.take() else {
            return;
        };
        let Some(indicator) = pending.replay(lease, self.presentation.geometry(), Instant::now())
        else {
            return;
        };
        self.presentation.agent_indicator = Some(pending);
        paint(lease.tab, lease.document_generation, &indicator);
    }
}

impl Pending {
    fn replay(
        self,
        lease: Lease,
        geometry: (u32, u32, u32),
        now: Instant,
    ) -> Option<ffi::AgentIndicator> {
        if self.tab != lease.tab
            || self.document != lease.document_generation
            || self.control_generation != lease.control_generation
            || lease.control_generation == 0
            || self.geometry != geometry
        {
            return None;
        }
        let ttl_ms = u32::try_from(self.expires.saturating_duration_since(now).as_millis()).ok()?;
        (ttl_ms > 0).then_some(ffi::AgentIndicator {
            ttl_ms,
            ..self.indicator
        })
    }
}

fn paint(native: u64, document: u64, indicator: &ffi::AgentIndicator) {
    // SAFETY: exact owned tab/current document on the CEF UI thread. Native
    // code independently requires a visible current viewport and clips pixels.
    // Presentation failure cannot change the outcome of an admitted input effect.
    let _ =
        unsafe { ffi::colossus_cef_presentation_agent_indicator(native, 1, document, indicator) };
}

fn geometry(model: &Value, viewport: (u32, u32), kind: u32) -> Option<ffi::AgentIndicator> {
    let (x, y) = super::center(model, viewport).ok()?;
    let model = model.get("model")?;
    let points = model
        .get("border")
        .or_else(|| model.get("content"))?
        .as_array()?;
    if points.len() != 8 {
        return None;
    }
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    for point in points.chunks_exact(2) {
        let px = point[0].as_f64()?;
        let py = point[1].as_f64()?;
        if !px.is_finite() || !py.is_finite() || px.abs() > 1_000_000.0 || py.abs() > 1_000_000.0 {
            return None;
        }
        left = left.min(px);
        top = top.min(py);
        right = right.max(px);
        bottom = bottom.max(py);
    }
    (right > left && bottom > top).then_some(ffi::AgentIndicator {
        kind,
        ttl_ms: 2200,
        x,
        y,
        left,
        top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_retains_only_the_exact_writer_document_geometry_and_original_deadline() {
        let now = Instant::now();
        let lease = Lease {
            tab: 1,
            session_generation: 1,
            control_generation: 7,
            viewport_generation: 3,
            document_generation: 5,
            pixel_width: 832,
            pixel_height: 520,
        };
        let pending = Pending {
            tab: 1,
            document: 5,
            control_generation: 7,
            geometry: (832, 520, 1000),
            indicator: ffi::AgentIndicator {
                kind: 1,
                ttl_ms: 2200,
                ..Default::default()
            },
            expires: now + Duration::from_millis(2200),
        };
        let recovered = Lease {
            viewport_generation: 4,
            ..lease
        };
        assert_eq!(
            pending
                .replay(
                    recovered,
                    (832, 520, 1000),
                    now + Duration::from_millis(900)
                )
                .unwrap()
                .ttl_ms,
            1300
        );
        for changed in [
            Lease { tab: 2, ..lease },
            Lease {
                document_generation: 6,
                ..lease
            },
            Lease {
                control_generation: 8,
                ..lease
            },
            Lease {
                control_generation: 0,
                ..lease
            },
        ] {
            assert!(pending.replay(changed, (832, 520, 1000), now).is_none());
        }
        assert!(pending.replay(recovered, (712, 440, 1000), now).is_none());
        assert!(pending.replay(recovered, (832, 520, 2000), now).is_none());
        assert!(
            pending
                .replay(
                    recovered,
                    (832, 520, 1000),
                    now + Duration::from_millis(2200)
                )
                .is_none()
        );
    }

    #[test]
    fn actual_click_anchor_and_border_are_retained_without_admitting_offscreen_targets() {
        let model = json!({"model": {
            "content": [12, 22, 108, 22, 108, 58, 12, 58],
            "border": [10, 20, 110, 20, 110, 60, 10, 60]
        }});
        let target = geometry(&model, (800, 600), 1).unwrap();
        assert_eq!((target.x, target.y), (60.0, 40.0));
        assert_eq!(
            (target.left, target.top, target.width, target.height),
            (10.0, 20.0, 100.0, 40.0)
        );
        assert!(geometry(&model, (40, 600), 1).is_none());
        assert!(geometry(&json!({"model":{"content":[0,0]}}), (800, 600), 1).is_none());
    }
}
