//! Human-only native document adoption; agent viewers await coordinator Snapshot.
use super::Host;
use colossus_browser_presentation::{Lease, PageState, PresentationError};
use colossus_contracts::{BrowserDocumentId, BrowserTarget};

#[derive(Clone, Copy)]
pub(super) enum Rejection {
    MissingActive,
    Disabled,
    Uninitialized,
    Cancelled,
    Lease,
    Control,
    Session,
    MissingSelection,
    MissingTab,
    NativeDocument,
    AdoptionDenied,
}

impl Host {
    pub(super) fn observation_stale(&self, reason: Rejection) -> PresentationError {
        #[cfg(all(windows, debug_assertions))]
        {
            let bit = 1_u16 << reason as u8;
            let observed = self.presentation.observation_diagnostics.get();
            if observed & bit == 0 {
                self.presentation
                    .observation_diagnostics
                    .set(observed | bit);
                let category = match reason {
                    Rejection::MissingActive => "missing_active",
                    Rejection::Disabled => "disabled",
                    Rejection::Uninitialized => "uninitialized",
                    Rejection::Cancelled => "cancelled",
                    Rejection::Lease => "lease_mismatch",
                    Rejection::Control => "control_mismatch",
                    Rejection::Session => "session_mismatch",
                    Rejection::MissingSelection => "missing_selection",
                    Rejection::MissingTab => "missing_tab",
                    Rejection::NativeDocument => "native_document_unavailable",
                    Rejection::AdoptionDenied => "unpublished_document",
                };
                eprintln!("EVIDENCE native-host-observe rejected={category}");
            }
        }
        #[cfg(not(all(windows, debug_assertions)))]
        let _ = reason;
        PresentationError::Stale
    }

    pub(super) fn observe_view(
        &mut self,
        lease: Lease,
        final_human_fence: bool,
    ) -> Result<PageState, PresentationError> {
        // Reading the same owned tab remains possible after hide, expiry or navigation;
        // no input or old pixels are authorized by this recovery metadata.
        self.owned_viewport(lease)?;
        let configured = self
            .presentation
            .active
            .as_ref()
            .ok_or_else(|| self.observation_stale(Rejection::MissingActive))?
            .0
            .target
            .tab_id
            .clone();
        let identity = if lease.control_generation > 0 {
            self.presentation
                .selected_tab
                .clone()
                .ok_or_else(|| self.observation_stale(Rejection::MissingSelection))?
        } else {
            // The native human path has exactly its original configured tab.
            self.owned_lease(lease)?;
            configured
        };
        let native = self
            .tabs
            .get(&identity)
            .ok_or_else(|| self.observation_stale(Rejection::MissingTab))?
            .native;
        let (origin, title, revision) = self
            .metadata(native)
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        let native_document = self
            .native_document(native)
            .map_err(|_| self.observation_stale(Rejection::NativeDocument))?;
        let tab = self
            .tabs
            .get_mut(&identity)
            .ok_or(PresentationError::Stale)?;
        let metadata_changed = revision != tab.revision;
        #[cfg(windows)]
        let metadata_changed = metadata_changed && self.presentation.control_generation == 0;
        // Windows Chrome can resume subframe loading after history restoration.
        // A read-only viewer may report that metadata while the committed main
        // document is unchanged. Keep the agent's revision, references and opaque
        // document untouched; only its normal Snapshot can refresh those fences.
        if metadata_changed || native_document != tab.native_document {
            // A viewer cannot invent the agent writer's next opaque document.
            // Only Snapshot's coordinator ticket may recover that ledger.
            if !crate::document::presentation_adoption(
                self.presentation.control_generation,
                self.presentation.human,
                final_human_fence,
            ) {
                return Err(self.observation_stale(Rejection::AdoptionDenied));
            }
            let mut bytes = [0_u8; 16];
            getrandom::fill(&mut bytes).map_err(|_| PresentationError::OutcomeUnknown)?;
            let token = bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            tab.summary.document_id = BrowserDocumentId::parse(format!("bd_{token}"))
                .map_err(|_| PresentationError::Invalid)?;
            tab.revision = revision;
            tab.native_document = native_document;
            tab.snapshot = None;
            tab.elements.clear();
        }
        tab.summary.origin = origin;
        tab.summary.title = title.clone();
        let target = BrowserTarget {
            tab_id: identity,
            document_id: tab.summary.document_id.clone(),
        };
        let (url, loading, history) = self
            .state(|state| {
                (
                    state.addresses.get(&native).cloned().unwrap_or_default(),
                    state.loading.contains(&native),
                    state.history.get(&native).copied().unwrap_or_default(),
                )
            })
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        let state = PageState {
            target,
            url,
            title,
            loading,
            can_go_back: history.0,
            can_go_forward: history.1,
        };
        state.validate()?;
        Ok(state)
    }
}
