//! Closed owned-page inspection, never a debugger endpoint or arbitrary protocol proxy.
use super::*;
impl Host {
    pub(super) fn developer_inspection(
        &mut self,
        native: u64,
        identity: &BrowserTabId,
        action: &BrowserAction,
        control: &BrowserDriverControl,
    ) -> Result<Value, BrowserDriverError> {
        if self
            .tabs
            .get(identity)
            .and_then(|tab| tab.snapshot.as_ref())
            != action.coordinate_snapshot()
        {
            return Err(BrowserDriverError::Stale);
        }
        validate_action(action)?;
        if self.interrupted(control) {
            return Err(BrowserDriverError::Cancelled);
        }
        // CSS enablement waits for asynchronous resource callbacks, which a
        // renderer pause can suspend. Complete fixed inspection-domain setup
        // first; the protected-field guard and actual read still share one
        // acknowledged pause, and no setup response is released to the caller.
        if let BrowserAction::Cdp { method, .. } = action {
            if matches!(
                method,
                colossus_contracts::BrowserCdpMethod::ComputedStyle
                    | colossus_contracts::BrowserCdpMethod::MediaQueries
            ) {
                // DOM.enable preserves frontend node IDs, unlike getDocument.
                self.method(native, "DOM.enable", json!({}), control)?;
                self.method(native, "CSS.enable", json!({}), control)?;
            }
            if matches!(
                method,
                colossus_contracts::BrowserCdpMethod::PerformanceMetrics
            ) {
                self.method(native, "Performance.enable", json!({}), control)?;
            }
        }
        self.callbacks
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .inspection_paused
            .remove(&native);
        // A document fence detects navigation, not DOM changes. Keep page
        // JavaScript paused across the private guard and read, and require the
        // renderer's paused event instead of treating a command ACK as a fence.
        let inspected = (|| {
            self.method(
                native,
                "Debugger.enable",
                json!({"maxScriptsCacheSize":0}),
                control,
            )?;
            self.method(native, "Debugger.pause", json!({}), control)?;
            self.trigger_inspection_pause(native)?;
            self.until(Some(control), Duration::from_secs(1), |state| {
                state.inspection_paused.contains(&native) || state.failed.contains(&native)
            })?;
            if !self.state(|state| state.inspection_paused.contains(&native))? {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            self.read_paused_inspection(native, action, control)
        })();
        // Cancellation cannot skip restoring the private debugger. An
        // unacknowledged restoration fences the session and releases no value.
        let restored = self.restore_inspection(native);
        if restored.is_err() {
            self.cancelled.store(true, Ordering::Release);
        }
        inspection_result(inspected, restored)
    }
    fn read_paused_inspection(
        &mut self,
        native: u64,
        action: &BrowserAction,
        control: &BrowserDriverControl,
    ) -> Result<Value, BrowserDriverError> {
        // Inspect private native attributes before allowing broader page reads.
        // Include shadow roots and child documents, including hidden fields.
        let document = self.method(
            native,
            "DOMSnapshot.getSnapshot",
            json!({"computedStyleWhitelist":[],"includeUserAgentShadowTree":true}),
            control,
        )?;
        protected_snapshot(&document)?;
        let result = match action {
            BrowserAction::Evaluate { expression, .. } => {
                let result=self.method(native,"Runtime.evaluate",json!({"expression":expression,"returnByValue":true,"throwOnSideEffect":true,"timeout":1000,"disableBreaks":true,"allowUnsafeEvalBlockedByCSP":false,"userGesture":false}),control)?;
                if result.get("exceptionDetails").is_some()
                    || result.pointer("/result/objectId").is_some()
                {
                    return Err(BrowserDriverError::Denied);
                }
                result
                    .pointer("/result/value")
                    .cloned()
                    .unwrap_or(Value::Null)
            }
            BrowserAction::Cdp { method, params, .. } => {
                self.method(native, method.as_str(), params.clone(), control)?
            }
            _ => return Err(BrowserDriverError::Unsupported),
        };
        self.check_document_fence()?;
        if !self.state(|state| state.inspection_paused.contains(&native))? {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let result = json!({"value":result});
        if serde_json::to_vec(&result).map_or(true, |bytes| bytes.len() > 32 * 1024) {
            return Err(BrowserDriverError::LimitExceeded);
        }
        Ok(result)
    }
    fn trigger_inspection_pause(&mut self, native: u64) -> Result<(), BrowserDriverError> {
        // Debugger.pause can wait for the next JS entry on a dormant page.
        // Dispatch only this fixed harmless expression, without awaiting its
        // result: it may remain pending until the debugger is disabled.
        let command = self.next_command;
        self.next_command = command
            .checked_add(1)
            .filter(|command| *command < i32::MAX)
            .ok_or(BrowserDriverError::LimitExceeded)?;
        let params = br#"{"expression":"void 0","returnByValue":true,"disableBreaks":false}"#;
        // SAFETY: exact owned tab, fixed native-only expression and borrowed
        // JSON storage on CEF's owning thread. Its result grants no authority.
        status_result(unsafe {
            ffi::colossus_cef_devtools(
                native,
                1,
                command,
                c"Runtime.evaluate".as_ptr(),
                params.as_ptr(),
                params.len(),
            )
        })
    }
    fn restore_inspection(&mut self, native: u64) -> Result<(), BrowserDriverError> {
        let command = self.next_command;
        self.next_command = command
            .checked_add(1)
            .filter(|command| *command < i32::MAX)
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        {
            let mut state = self
                .callbacks
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            state.expected = Some(command);
            state.result = None;
        }
        // This exact fixed cleanup is independent of request cancellation.
        // Debugger.disable removes pending pauses and resumes a paused program.
        // SAFETY: exact owned tab/generation and fixed empty JSON on CEF UI thread.
        let dispatched = status_result(unsafe {
            ffi::colossus_cef_devtools(
                native,
                1,
                command,
                c"Debugger.disable".as_ptr(),
                b"{}".as_ptr(),
                2,
            )
        });
        let completed = dispatched.and_then(|()| {
            self.until(None, Duration::from_secs(1), |state| {
                state.result.is_some() || state.failed.contains(&native)
            })
        });
        let mut state = self
            .callbacks
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        state.expected = None;
        let result = state.result.take();
        completed?;
        if state.failed.contains(&native) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        result.ok_or(BrowserDriverError::OutcomeUnknown)??;
        state.inspection_paused.remove(&native);
        Ok(())
    }
}
fn inspection_result(
    inspected: Result<Value, BrowserDriverError>,
    restored: Result<(), BrowserDriverError>,
) -> Result<Value, BrowserDriverError> {
    restored.map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    inspected
}
fn validate_action(action: &BrowserAction) -> Result<(), BrowserDriverError> {
    match action {
        BrowserAction::Evaluate { expression, .. } => {
            if expression.is_empty() || expression.len() > 8192 {
                return Err(BrowserDriverError::Denied);
            }
        }
        BrowserAction::Cdp { method, params, .. } => {
            if !params.is_object()
                || serde_json::to_vec(params).map_or(true, |bytes| bytes.len() > 16 * 1024)
            {
                return Err(BrowserDriverError::Denied);
            }
            // No caller-selected execution context, target, remote object,
            // file, network envelope or debugger session crosses this boundary.
            let allowed: &[&str] = match method {
                colossus_contracts::BrowserCdpMethod::Document => &["depth", "pierce"],
                colossus_contracts::BrowserCdpMethod::QuerySelector
                | colossus_contracts::BrowserCdpMethod::QuerySelectorAll => &["nodeId", "selector"],
                colossus_contracts::BrowserCdpMethod::Attributes
                | colossus_contracts::BrowserCdpMethod::BoxModel
                | colossus_contracts::BrowserCdpMethod::OuterHtml
                | colossus_contracts::BrowserCdpMethod::ComputedStyle => &["nodeId"],
                _ => &[],
            };
            if params
                .as_object()
                .is_none_or(|params| params.keys().any(|key| !allowed.contains(&key.as_str())))
            {
                return Err(BrowserDriverError::Denied);
            }
        }
        _ => return Err(BrowserDriverError::Unsupported),
    }
    Ok(())
}
// DOM.getDocument resets the frontend node-ID namespace. The private guard
// uses a structured DOMSnapshot with complete attribute names and values,
// preserving IDs issued by admitted CDP inspection. The value envelope also
// preserves a JSON null result across the optional observation field.
fn protected_snapshot(value: &Value) -> Result<(), BrowserDriverError> {
    let nodes = value["domNodes"]
        .as_array()
        .ok_or(BrowserDriverError::Denied)?;
    if nodes.len() > 100_000 {
        return Err(BrowserDriverError::LimitExceeded);
    }
    for node in nodes {
        // One acknowledged renderer pause does not establish coverage of a
        // separately executing child renderer. Reject embedding elements even
        // before they have an attached content document.
        if node.get("contentDocumentIndex").is_some()
            || node["nodeName"].as_str().is_some_and(|name| {
                ["iframe", "frame", "object", "embed"]
                    .iter()
                    .any(|element| name.eq_ignore_ascii_case(element))
            })
        {
            return Err(BrowserDriverError::Denied);
        }
        if let Some(attributes) = node.get("attributes") {
            let attributes = attributes.as_array().ok_or(BrowserDriverError::Denied)?;
            if attributes.len() > 2048 {
                return Err(BrowserDriverError::LimitExceeded);
            }
            let mut pairs = Vec::with_capacity(attributes.len() * 2);
            for attribute in attributes {
                pairs.push(
                    attribute["name"]
                        .as_str()
                        .ok_or(BrowserDriverError::Denied)?
                        .to_owned(),
                );
                pairs.push(
                    attribute["value"]
                        .as_str()
                        .ok_or(BrowserDriverError::Denied)?
                        .to_owned(),
                );
            }
            if semantic::protected(&pairs) {
                return Err(BrowserDriverError::Denied);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn broader_inspection_rejects_hidden_shadow_and_child_document_credentials() {
        assert!(matches!(
            protected_snapshot(
                &json!({"domNodes":[{"attributes":[{"name":"type","value":"password"}]}]})
            ),
            Err(BrowserDriverError::Denied)
        ));
        assert!(matches!(
            protected_snapshot(&json!({"domNodes":[{"attributes":[{"name":"type","value":42}]}]})),
            Err(BrowserDriverError::Denied)
        ));
        assert!(
            protected_snapshot(
                &json!({"domNodes":[{"attributes":[{"name":"type","value":"search"}]}]})
            )
            .is_ok()
        );
        for attributes in [
            json!([{"name":"type","value":"password"}]),
            json!([{"name":"name","value":"api_token"}]),
            json!([{"name":"autocomplete","value":"current-password"}]),
        ] {
            assert!(matches!(
                protected_snapshot(&json!({"domNodes":[{"attributes":attributes}]})),
                Err(BrowserDriverError::Denied)
            ));
        }
    }
    #[test]
    fn inspection_values_remain_private_until_debugger_restoration_is_acknowledged() {
        assert_eq!(inspection_result(Ok(json!(42)), Ok(())), Ok(json!(42)));
        for error in [
            BrowserDriverError::Denied,
            BrowserDriverError::OutcomeUnknown,
        ] {
            assert_eq!(
                inspection_result(Ok(json!({"private":"not released"})), Err(error)),
                Err(BrowserDriverError::OutcomeUnknown)
            );
            assert_eq!(
                inspection_result(Err(BrowserDriverError::Denied), Err(error)),
                Err(BrowserDriverError::OutcomeUnknown)
            );
        }
        assert_eq!(
            inspection_result(Err(BrowserDriverError::Denied), Ok(())),
            Err(BrowserDriverError::Denied)
        );
    }
    #[test]
    fn one_renderer_pause_does_not_admit_any_child_document_layout() {
        for node in [
            json!({"nodeName":"IFRAME"}),
            json!({"nodeName":"frame"}),
            json!({"nodeName":"OBJECT"}),
            json!({"nodeName":"EMBED"}),
            json!({"nodeName":"DIV","contentDocumentIndex":0}),
        ] {
            assert_eq!(
                protected_snapshot(&json!({"domNodes":[node]})),
                Err(BrowserDriverError::Denied)
            );
        }
        assert!(protected_snapshot(&json!({"domNodes":[{"nodeName":"DIV"}]})).is_ok());
    }
}
