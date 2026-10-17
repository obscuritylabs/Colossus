//! Read-only snapshots can recover an independently changed owned document.
use colossus_contracts::{BrowserAction, BrowserDocumentId};
use colossus_ports::BrowserDriverError;

pub(super) fn snapshot_document(
    action: &BrowserAction,
    stored: &BrowserDocumentId,
    requested: &BrowserDocumentId,
    next: &BrowserDocumentId,
    changed: bool,
) -> Result<BrowserDocumentId, BrowserDriverError> {
    if stored != requested {
        return Err(BrowserDriverError::Stale);
    }
    if changed {
        if !matches!(action, BrowserAction::Snapshot { .. }) {
            return Err(BrowserDriverError::Stale);
        }
        if next == stored {
            return Err(BrowserDriverError::Denied);
        }
        Ok(next.clone())
    } else {
        Ok(stored.clone())
    }
}

/// Initial human metadata, or its final fenced receipt, may rotate the native
/// human ledger. A read-only agent viewer never owns coordinator document tickets.
pub(super) fn presentation_adoption(generation: u64, human: bool, final_fence: bool) -> bool {
    generation == 0 && (human || final_fence)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(byte: char) -> BrowserDocumentId {
        BrowserDocumentId::parse(format!("bd_{}", byte.to_string().repeat(32))).unwrap()
    }
    #[test]
    fn read_only_observation_cannot_adopt_a_writer_document() {
        assert!(presentation_adoption(0, true, false));
        assert!(presentation_adoption(0, false, true));
        assert!(!presentation_adoption(0, false, false));
        assert!(!presentation_adoption(1, false, false));
        assert!(!presentation_adoption(1, true, true));
    }
    #[test]
    fn actual_changed_document_can_only_advance_a_fresh_read_snapshot() {
        let (old, next) = (doc('1'), doc('2'));
        assert_eq!(
            snapshot_document(
                &BrowserAction::Snapshot { max_nodes: 32 },
                &old,
                &old,
                &next,
                true
            ),
            Ok(next.clone())
        );
        assert_eq!(
            snapshot_document(&BrowserAction::Back {}, &old, &old, &next, true),
            Err(BrowserDriverError::Stale)
        );
        assert_eq!(
            snapshot_document(
                &BrowserAction::Screenshot { max_bytes: 100 },
                &old,
                &old,
                &next,
                true
            ),
            Err(BrowserDriverError::Stale)
        );
        assert_eq!(
            snapshot_document(
                &BrowserAction::Snapshot { max_nodes: 32 },
                &old,
                &next,
                &doc('3'),
                true
            ),
            Err(BrowserDriverError::Stale)
        );
        assert_eq!(
            snapshot_document(
                &BrowserAction::Snapshot { max_nodes: 32 },
                &old,
                &old,
                &old,
                true
            ),
            Err(BrowserDriverError::Denied)
        );
        assert_eq!(
            snapshot_document(
                &BrowserAction::Snapshot { max_nodes: 32 },
                &old,
                &old,
                &next,
                false
            ),
            Ok(old)
        );
    }
}
