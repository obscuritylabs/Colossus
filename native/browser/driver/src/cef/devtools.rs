//! Bounded native acknowledgements for one command or one complete input gesture.
use super::*;

#[cfg(test)]
#[path = "devtools_tests.rs"]
mod tests;

pub(super) struct Pending {
    pub(super) tab: u64,
    pub(super) command: i32,
    pub(super) result: Option<Result<Value, BrowserDriverError>>,
}

impl Host {
    /// Admit a press/release pair against one document before pumping events.
    /// Navigation caused by the press must not prevent its matching release.
    /// Every subsequent operation still checks the original document fence.
    pub(super) fn dispatch<const N: usize>(
        &mut self,
        tab: u64,
        method: &str,
        params: [Value; N],
        control: &BrowserDriverControl,
    ) -> Result<Vec<Value>, BrowserDriverError> {
        if N == 0
            || N > 2
            || (N == 2
                && !matches!(
                    method,
                    "Input.dispatchKeyEvent" | "Input.dispatchMouseEvent"
                ))
        {
            return Err(BrowserDriverError::Denied);
        }
        if self.interrupted(control) {
            return Err(BrowserDriverError::Cancelled);
        }
        self.check_document_fence()?;
        #[cfg(windows)]
        if method.starts_with("Input.") || method == "DOM.focus" {
            let document = self.native_document(tab)?;
            // SAFETY: fixed agent operation admitted on the owning thread for
            // this exact document. Native human input remains independently denied.
            status_result(unsafe { ffi::colossus_cef_automation_activate(tab, 1, document) })?;
        }
        let params: Vec<_> = params
            .iter()
            .map(serde_json::to_vec)
            .collect::<Result<_, _>>()
            .map_err(|_| BrowserDriverError::Failed)?;
        let method = CString::new(method).map_err(|_| BrowserDriverError::Failed)?;
        let first = self.next_command;
        self.next_command = first
            .checked_add(N as i32)
            .filter(|command| *command < i32::MAX)
            .ok_or(BrowserDriverError::LimitExceeded)?;
        {
            let mut state = self
                .callbacks
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            state.pending = (first..self.next_command)
                .map(|command| Pending {
                    tab,
                    command,
                    result: None,
                })
                .collect();
        }
        let acknowledged = (|| {
            for (offset, params) in params.iter().enumerate() {
                // SAFETY: private owned tab, fixed method, and bytes retained for
                // the call. Submit the complete bounded gesture without pumping.
                let status = unsafe {
                    ffi::colossus_cef_devtools(
                        tab,
                        1,
                        first + offset as i32,
                        method.as_ptr(),
                        params.as_ptr(),
                        params.len(),
                    )
                };
                if offset == 0 {
                    status_result(status)?;
                } else if status != 0 {
                    return Err(BrowserDriverError::OutcomeUnknown);
                }
            }
            self.until(Some(control), Duration::from_secs(30), |state| {
                state.pending.iter().all(|pending| pending.result.is_some())
                    || state.failed.contains(&tab)
            })
        })();
        let mut state = self
            .callbacks
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let pending = std::mem::take(&mut state.pending);
        acknowledged?;
        if state.failed.contains(&tab) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        pending
            .into_iter()
            .map(|pending| pending.result.ok_or(BrowserDriverError::OutcomeUnknown)?)
            .collect()
    }
}
