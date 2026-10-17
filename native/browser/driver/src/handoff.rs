//! One-shot proof retained inside the native host after irreversible human fencing.
use colossus_contracts::{BrowserSessionBinding, BrowserSessionId, BrowserTarget};
use colossus_ports::{BrowserDriverError, BrowserNativeHandoffRequest};

struct Admission {
    binding: BrowserSessionBinding,
    session_id: BrowserSessionId,
    target: BrowserTarget,
}

#[derive(Default)]
pub struct Cache {
    admission: Option<Admission>,
    receipt: Option<BrowserNativeHandoffRequest>,
}

impl Cache {
    pub fn admit(
        &mut self,
        binding: BrowserSessionBinding,
        session_id: BrowserSessionId,
        target: BrowserTarget,
    ) {
        self.admission = Some(Admission {
            binding,
            session_id,
            target,
        });
        self.receipt = None;
    }

    pub fn record(
        &mut self,
        target: BrowserTarget,
        native_document_generation: u64,
    ) -> Result<(), BrowserDriverError> {
        let admission = self.admission.as_ref().ok_or(BrowserDriverError::Stale)?;
        if native_document_generation == 0 || target.tab_id != admission.target.tab_id {
            return Err(BrowserDriverError::Stale);
        }
        self.receipt = Some(BrowserNativeHandoffRequest {
            binding: admission.binding.clone(),
            session_id: admission.session_id.clone(),
            expected_target: admission.target.clone(),
            confirmed_target: target,
            native_document_generation,
        });
        Ok(())
    }

    pub fn consume(
        &mut self,
        request: &BrowserNativeHandoffRequest,
        current_target: &BrowserTarget,
        current_native_document: u64,
    ) -> Result<(), BrowserDriverError> {
        if self.receipt.as_ref() != Some(request)
            || &request.confirmed_target != current_target
            || request.native_document_generation != current_native_document
            || current_native_document == 0
        {
            return Err(BrowserDriverError::Stale);
        }
        self.receipt = None;
        self.admission = None;
        Ok(())
    }

    pub fn revoke(&mut self) {
        self.admission = None;
        self.receipt = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{BrowserDocumentId, BrowserScope, BrowserTabId};

    fn request() -> BrowserNativeHandoffRequest {
        let tab_id = BrowserTabId::parse("bt_0123456789abcdef0123456789abcdef").unwrap();
        BrowserNativeHandoffRequest {
            binding: BrowserSessionBinding {
                runtime_id: "runtime".into(),
                workspace_id: "workspace".into(),
                application_id: "application".into(),
                scope: BrowserScope::Conversation {
                    id: "conversation".into(),
                },
            },
            session_id: BrowserSessionId::parse("bs_0123456789abcdef0123456789abcdef").unwrap(),
            expected_target: BrowserTarget {
                tab_id: tab_id.clone(),
                document_id: BrowserDocumentId::parse("bd_0123456789abcdef0123456789abcdef")
                    .unwrap(),
            },
            confirmed_target: BrowserTarget {
                tab_id,
                document_id: BrowserDocumentId::parse("bd_fedcba9876543210fedcba9876543210")
                    .unwrap(),
            },
            native_document_generation: 2,
        }
    }

    fn cache(request: &BrowserNativeHandoffRequest) -> Cache {
        let mut cache = Cache::default();
        cache.admit(
            request.binding.clone(),
            request.session_id.clone(),
            request.expected_target.clone(),
        );
        cache
            .record(
                request.confirmed_target.clone(),
                request.native_document_generation,
            )
            .unwrap();
        cache
    }

    #[test]
    fn same_origin_document_change_requires_exact_original_admission_and_native_fence() {
        let request = request();
        let mut cache = cache(&request);
        let mut forged = request.clone();
        forged.binding.workspace_id = "foreign-workspace".into();
        assert_eq!(
            cache.consume(&forged, &request.confirmed_target, 2),
            Err(BrowserDriverError::Stale)
        );
        forged = request.clone();
        forged.expected_target = request.confirmed_target.clone();
        assert_eq!(
            cache.consume(&forged, &request.confirmed_target, 2),
            Err(BrowserDriverError::Stale)
        );
        assert_eq!(
            cache.consume(&request, &request.expected_target, 2),
            Err(BrowserDriverError::Stale)
        );
        assert_eq!(
            cache.consume(&request, &request.confirmed_target, 3),
            Err(BrowserDriverError::Stale)
        );
        assert_eq!(
            cache.consume(&request, &request.confirmed_target, 2),
            Ok(())
        );
        assert_eq!(
            cache.consume(&request, &request.confirmed_target, 2),
            Err(BrowserDriverError::Stale)
        );
    }

    #[test]
    fn writer_or_close_revocation_drops_pending_fence_permanently() {
        let request = request();
        let mut cache = cache(&request);
        cache.revoke();
        assert_eq!(
            cache.consume(&request, &request.confirmed_target, 2),
            Err(BrowserDriverError::Stale)
        );
        assert_eq!(
            cache.record(request.confirmed_target.clone(), 2),
            Err(BrowserDriverError::Stale)
        );
    }
}
