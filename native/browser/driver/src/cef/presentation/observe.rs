//! Human-only native document adoption; agent viewers await coordinator Snapshot.
use super::Host;
use colossus_browser_presentation::{Lease, PageState, PresentationError};
use colossus_contracts::{BrowserDocumentId, BrowserTarget};
impl Host {
    pub(super) fn observe_view(
        &mut self,
        lease: Lease,
        final_human_fence: bool,
    ) -> Result<PageState, PresentationError> {
        // Reading the same owned tab remains possible after hide, expiry or navigation;
        // no input or old pixels are authorized by this recovery metadata.
        self.owned_lease(lease)?;
        let identity = self
            .presentation
            .active
            .as_ref()
            .ok_or(PresentationError::Stale)?
            .0
            .target
            .tab_id
            .clone();
        let (origin, title, revision) = self
            .metadata(lease.tab)
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        let native_document = self
            .native_document(lease.tab)
            .map_err(|_| PresentationError::Stale)?;
        let tab = self
            .tabs
            .get_mut(&identity)
            .ok_or(PresentationError::Stale)?;
        if revision != tab.revision || native_document != tab.native_document {
            // A viewer cannot invent the agent writer's next opaque document.
            // Only Snapshot's coordinator ticket may recover that ledger.
            if !crate::document::presentation_adoption(
                self.presentation.control_generation,
                self.presentation.human,
                final_human_fence,
            ) {
                return Err(PresentationError::Stale);
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
                    state.addresses.get(&lease.tab).cloned().unwrap_or_default(),
                    state.loading.contains(&lease.tab),
                    state.history.get(&lease.tab).copied().unwrap_or_default(),
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
