use super::*;

impl Runtime {
    /// Read one bounded newest-first page from the rebuildable session activity projection.
    pub fn session_activity_page(
        &self,
        session_id: &str,
        after_key: Option<&str>,
        limit: usize,
    ) -> Result<ProjectedSessionActivityPage, RuntimeError> {
        self.session_activity
            .list_page(session_id, after_key, limit)
            .map_err(Into::into)
    }
}
