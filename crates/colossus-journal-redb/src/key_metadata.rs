//! Read-only encrypted-header discovery for explicitly reviewed custody rewraps.
use super::*;
use std::collections::BTreeSet;

/// Public key identifiers only; no payloads, keys, signing seeds or anchors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalKeyMetadata {
    /// Unique encryption identifiers referenced by retained public payload headers.
    pub encryption_key_ids: Vec<String>,
    /// Signing identifier of the retained latest checkpoint, if one exists.
    pub checkpoint_key_id: Option<String>,
}

/// Inspect frozen existing redb metadata without opening a key provider, creating
/// files, decrypting payloads or repairing the journal. An active writer fails.
pub fn journal_key_metadata(path: &Path) -> Result<JournalKeyMetadata, StoreError> {
    #[derive(Deserialize)]
    struct Header<'a> {
        #[serde(borrow)]
        payload: PayloadHeader<'a>,
    }
    #[derive(Deserialize)]
    struct PayloadHeader<'a> {
        algorithm: &'a str,
        key_id: &'a str,
    }
    let database = redb::ReadOnlyDatabase::open(path).map_err(|_| {
        StoreError::Adapter("journal metadata source is unavailable or requires recovery".into())
    })?;
    let transaction = database
        .begin_read()
        .map_err(|_| StoreError::Adapter("journal metadata is unavailable".into()))?;
    let events = transaction
        .open_table(EVENTS)
        .map_err(|_| StoreError::Verification("journal event metadata is missing".into()))?;
    let mut ids = BTreeSet::new();
    for (position, entry) in events
        .iter()
        .map_err(|_| StoreError::Adapter("journal metadata is unavailable".into()))?
        .enumerate()
    {
        if position >= 1_000_000 {
            return Err(StoreError::Verification(
                "journal metadata scan exceeds bound".into(),
            ));
        }
        let (_, value) =
            entry.map_err(|_| StoreError::Adapter("journal metadata is unavailable".into()))?;
        if value.value().len() > 8 * 1024 * 1024 {
            return Err(StoreError::Verification(
                "journal envelope exceeds metadata bound".into(),
            ));
        }
        let header: Header<'_> = serde_json::from_slice(value.value())
            .map_err(|_| StoreError::Verification("journal payload metadata is invalid".into()))?;
        match header.payload.algorithm {
            ENCRYPTED_PAYLOAD_ALGORITHM => {
                validate_id(header.payload.key_id)?;
                ids.insert(header.payload.key_id.to_owned());
                if ids.len() > 256 {
                    return Err(StoreError::Verification(
                        "journal key metadata exceeds bound".into(),
                    ));
                }
            }
            PLAINTEXT_PAYLOAD_ALGORITHM if header.payload.key_id == "none" => {}
            _ => {
                return Err(StoreError::Verification(
                    "journal payload protection is unsupported".into(),
                ));
            }
        }
    }
    let metadata = transaction
        .open_table(METADATA)
        .map_err(|_| StoreError::Verification("journal metadata is missing".into()))?;
    let checkpoint_key_id = metadata
        .get("latest_checkpoint")
        .map_err(|_| StoreError::Verification("journal checkpoint metadata is invalid".into()))?
        .map(|bytes| {
            if bytes.value().len() > 4096 {
                return Err(StoreError::Verification(
                    "journal checkpoint metadata exceeds bound".into(),
                ));
            }
            let checkpoint: SignedCheckpoint =
                serde_json::from_slice(bytes.value()).map_err(|_| {
                    StoreError::Verification("journal checkpoint metadata is invalid".into())
                })?;
            validate_id(&checkpoint.key_id)?;
            Ok(checkpoint.key_id)
        })
        .transpose()?;
    Ok(JournalKeyMetadata {
        encryption_key_ids: ids.into_iter().collect(),
        checkpoint_key_id,
    })
}
fn validate_id(id: &str) -> Result<(), StoreError> {
    if id.is_empty() || id.len() > 256 || id.trim() != id || id.chars().any(char::is_control) {
        Err(StoreError::Verification(
            "journal key identifier is invalid".into(),
        ))
    } else {
        Ok(())
    }
}
