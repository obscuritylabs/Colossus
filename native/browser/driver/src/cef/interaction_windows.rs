//! Snapshot-fenced guest pointer operations. No system cursor or public CDP.
use super::*;
use colossus_contracts::{BrowserMouseButton, BrowserPoint, BrowserViewport};

impl Host {
    pub(super) fn rich_interaction(
        &mut self,
        native: u64,
        identity: &BrowserTabId,
        action: &BrowserAction,
        control: &BrowserDriverControl,
    ) -> Result<(), BrowserDriverError> {
        if let Some(snapshot) = action.coordinate_snapshot()
            && self
                .tabs
                .get(identity)
                .and_then(|tab| tab.snapshot.as_ref())
                != Some(snapshot)
        {
            return Err(BrowserDriverError::Stale);
        }
        match action {
            BrowserAction::Hover { element }
            | BrowserAction::DoubleClick { element }
            | BrowserAction::RightClick { element } => {
                let (model, (x, y)) =
                    self.pointer_target(native, identity, element, true, control)?;
                self.pointer_event(native, "mouseMoved", x, y, "none", 0, 0, control)?;
                if !matches!(action, BrowserAction::Hover { .. }) {
                    let (button, clicks) = if matches!(action, BrowserAction::RightClick { .. }) {
                        ("right", 1)
                    } else {
                        ("left", 2)
                    };
                    self.pointer_click(native, x, y, button, clicks, control)?;
                }
                self.indicate_agent_target(native, &model, 1);
            }
            BrowserAction::Drag { element, to } => {
                // Both endpoints must already fit the same viewport. Scrolling
                // one independently would invalidate the other endpoint.
                let (_, start) = self.pointer_target(native, identity, element, false, control)?;
                let (_, end) = self.pointer_target(native, identity, to, false, control)?;
                let path: Vec<_> = (0..=16)
                    .map(|step| {
                        let t = f64::from(step) / 16.0;
                        (
                            start.0 + (end.0 - start.0) * t,
                            start.1 + (end.1 - start.1) * t,
                        )
                    })
                    .collect();
                self.pointer_drag(native, &path, control)?;
                self.indicate_pointer_point(native, end.0, end.1);
            }
            BrowserAction::ClickAt {
                viewport,
                point,
                button,
                click_count,
                ..
            } => {
                let scale = self.coordinate_scale(*viewport)?;
                let (x, y) = scaled_point(*point, *viewport, scale)?;
                let button = match button {
                    BrowserMouseButton::Left => "left",
                    BrowserMouseButton::Middle => "middle",
                    BrowserMouseButton::Right => "right",
                };
                if !(1..=2).contains(click_count) {
                    return Err(BrowserDriverError::Denied);
                }
                self.pointer_click(native, x, y, button, *click_count, control)?;
                self.indicate_pointer_point(native, x, y);
            }
            BrowserAction::DragAt { viewport, path, .. } => {
                let scale = self.coordinate_scale(*viewport)?;
                if !(2..=64).contains(&path.len()) {
                    return Err(BrowserDriverError::Denied);
                }
                let path: Result<Vec<_>, _> = path
                    .iter()
                    .map(|point| scaled_point(*point, *viewport, scale))
                    .collect();
                let path = path?;
                self.pointer_drag(native, &path, control)?;
                if let Some(&(x, y)) = path.last() {
                    self.indicate_pointer_point(native, x, y);
                }
            }
            BrowserAction::Annotate {
                viewport,
                annotations,
                ttl_ms,
                ..
            } => {
                let scale = self.coordinate_scale(*viewport)?;
                self.annotate(native, identity, annotations, *ttl_ms, scale, control)?;
            }
            BrowserAction::AnnotationClear {} => self.clear_annotations(native),
            _ => return Err(BrowserDriverError::Unsupported),
        }
        Ok(())
    }
    fn indicate_pointer_point(&mut self, native: u64, x: f64, y: f64) {
        self.indicate_agent_target(
            native,
            &json!({"model":{"content":[x-0.5,y-0.5,x+0.5,y-0.5,x+0.5,y+0.5,x-0.5,y+0.5]}}),
            1,
        );
    }

    pub(super) fn coordinate_scale(
        &self,
        viewport: BrowserViewport,
    ) -> Result<f64, BrowserDriverError> {
        let (width, height, milli) = self.presentation.geometry();
        let scale = f64::from(milli) / 1000.0;
        if viewport.width == 0
            || viewport.height == 0
            || f64::from(viewport.width) != (f64::from(width) * scale).ceil()
            || f64::from(viewport.height) != (f64::from(height) * scale).ceil()
        {
            return Err(BrowserDriverError::Stale);
        }
        Ok(scale)
    }

    pub(super) fn pointer_target(
        &mut self,
        native: u64,
        identity: &BrowserTabId,
        element: &BrowserElementRef,
        scroll: bool,
        control: &BrowserDriverControl,
    ) -> Result<(Value, (f64, f64)), BrowserDriverError> {
        let backend = self.backend(identity, element)?;
        if scroll {
            self.method(
                native,
                "DOM.scrollIntoViewIfNeeded",
                json!({"backendNodeId":backend}),
                control,
            )?;
        }
        let model = self.method(
            native,
            "DOM.getBoxModel",
            json!({"backendNodeId":backend}),
            control,
        )?;
        let point = center(&model, self.viewport_dimensions())?;
        Ok((model, point))
    }

    #[allow(clippy::too_many_arguments)]
    fn pointer_event(
        &mut self,
        native: u64,
        kind: &str,
        x: f64,
        y: f64,
        button: &str,
        buttons: u8,
        count: u8,
        control: &BrowserDriverControl,
    ) -> Result<(), BrowserDriverError> {
        self.method(
            native,
            "Input.dispatchMouseEvent",
            json!({"type":kind,"x":x,"y":y,"button":button,"buttons":buttons,"clickCount":count}),
            control,
        )?;
        Ok(())
    }

    fn reset_pointer(&self, native: u64, x: f64, y: f64, button: &str) {
        let document = self
            .expected_revision
            .map_or(0, |(_, _, document)| document);
        let button = match button {
            "middle" => 1,
            "right" => 2,
            _ => 0,
        };
        // SAFETY: cleanup only releases an admitted gesture. Native code checks
        // the exact old document, so cancellation never targets a replacement page.
        let _ = unsafe {
            ffi::colossus_cef_automation_pointer_reset(native, 1, document, x, y, button)
        };
    }

    fn pointer_click(
        &mut self,
        native: u64,
        x: f64,
        y: f64,
        button: &str,
        count: u8,
        control: &BrowserDriverControl,
    ) -> Result<(), BrowserDriverError> {
        let mask = match button {
            "right" => 2,
            "middle" => 4,
            _ => 1,
        };
        for click in 1..=count {
            let result = self
                .pointer_event(native, "mousePressed", x, y, button, mask, click, control)
                .and_then(|()| {
                    self.pointer_event(native, "mouseReleased", x, y, button, 0, click, control)
                });
            if result.is_err() {
                self.reset_pointer(native, x, y, button);
            }
            result?;
        }
        Ok(())
    }

    fn pointer_drag(
        &mut self,
        native: u64,
        path: &[(f64, f64)],
        control: &BrowserDriverControl,
    ) -> Result<(), BrowserDriverError> {
        let &(x, y) = path.first().ok_or(BrowserDriverError::Denied)?;
        self.callbacks
            .state
            .lock()
            .map_err(|_| BrowserDriverError::Failed)?
            .drag_data
            .remove(&native);
        let mut pointer = (x, y);
        let result = (|| {
            self.method(
                native,
                "Input.setInterceptDrags",
                json!({"enabled":true}),
                control,
            )?;
            self.pointer_event(native, "mouseMoved", x, y, "none", 0, 0, control)?;
            self.pointer_event(native, "mousePressed", x, y, "left", 1, 1, control)?;
            let mut data = None;
            for &(x, y) in &path[1..] {
                pointer = (x, y);
                if let Some(data) = &data {
                    self.method(
                        native,
                        "Input.dispatchDragEvent",
                        json!({"type":"dragOver","x":x,"y":y,"data":data}),
                        control,
                    )?;
                } else {
                    self.pointer_event(native, "mouseMoved", x, y, "left", 1, 0, control)?;
                    data = self
                        .callbacks
                        .state
                        .lock()
                        .map_err(|_| BrowserDriverError::Failed)?
                        .drag_data
                        .remove(&native);
                    if let Some(data) = &data {
                        if data
                            .get("files")
                            .and_then(Value::as_array)
                            .is_some_and(|files| !files.is_empty())
                        {
                            return Err(BrowserDriverError::Denied);
                        }
                        self.method(
                            native,
                            "Input.dispatchDragEvent",
                            json!({"type":"dragEnter","x":x,"y":y,"data":data}),
                            control,
                        )?;
                    }
                }
            }
            let (x, y) = pointer;
            if let Some(data) = data {
                self.method(
                    native,
                    "Input.dispatchDragEvent",
                    json!({"type":"drop","x":x,"y":y,"data":data}),
                    control,
                )?;
            }
            self.pointer_event(native, "mouseReleased", x, y, "left", 0, 1, control)?;
            self.method(
                native,
                "Input.setInterceptDrags",
                json!({"enabled":false}),
                control,
            )?;
            Ok(())
        })();
        if result.is_err() {
            // Cleanup bypasses the cancelled request control but keeps its old
            // document fence, releasing the pointer and cancelling interception
            // even when the first move or the final disable was interrupted.
            self.reset_pointer(native, pointer.0, pointer.1, "left");
        }
        result
    }
}

fn scaled_point(
    point: BrowserPoint,
    viewport: BrowserViewport,
    scale: f64,
) -> Result<(f64, f64), BrowserDriverError> {
    if point.x >= viewport.width || point.y >= viewport.height {
        return Err(BrowserDriverError::Denied);
    }
    Ok((f64::from(point.x) / scale, f64::from(point.y) / scale))
}
