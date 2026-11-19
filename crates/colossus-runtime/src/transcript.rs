//! Canonical conversation export without provider calls or activity projections.

use crate::{Runtime, RuntimeError};
use colossus_contracts::{TranscriptActivity, TranscriptPage};
use colossus_session::CanonicalTranscriptReader;

impl Runtime {
    /// Read a byte- and work-bounded chronological page through a fixed canonical
    /// session snapshot. Private cursors carry no application disclosure authority.
    pub fn read_session_transcript(
        &self,
        session_id: &str,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<TranscriptPage, RuntimeError> {
        let activity = if self
            .native_session_lifecycle
            .has_live_primary_run(session_id)?
        {
            TranscriptActivity::Active
        } else {
            TranscriptActivity::Idle
        };
        CanonicalTranscriptReader::new(self.journal())
            .read(session_id, cursor, page_size, activity)
            .map_err(Into::into)
    }
}
