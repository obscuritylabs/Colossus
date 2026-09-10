use colossus_contracts::SessionSummary;
use colossus_ports::StoreError;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(super) fn recent_sessions(
    sessions: Vec<SessionSummary>,
    limit: usize,
) -> Result<Vec<SessionSummary>, StoreError> {
    // RFC3339 permits variable fractional precision and offsets. Its strings
    // are not chronologically sortable (for example, .1Z sorts after .100001Z).
    let mut sessions = sessions
        .into_iter()
        .map(|session| {
            OffsetDateTime::parse(&session.updated_at, &Rfc3339)
                .map(|timestamp| (timestamp, session))
                .map_err(|_| StoreError::Verification("session update timestamp is invalid".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    sessions.sort_by(|(left_time, left), (right_time, right)| {
        right_time
            .cmp(left_time)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(sessions
        .into_iter()
        .take(limit)
        .map(|(_, session)| session)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, updated_at: &str) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            title: None,
            created_at: updated_at.into(),
            updated_at: updated_at.into(),
            message_count: 0,
            last_run_id: None,
            last_user_preview: None,
        }
    }

    #[test]
    fn fractional_precision_and_offsets_sort_by_instant_not_spelling() {
        let sessions = vec![
            session("older", "2026-09-09T00:00:00.1Z"),
            session("newer", "2026-09-09T00:00:00.100001Z"),
            session("offset", "2026-09-09T02:00:00.09+02:00"),
        ];
        let ordered = recent_sessions(sessions, 2).unwrap();
        assert_eq!(
            ordered
                .iter()
                .map(|value| value.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "older"]
        );
    }

    #[test]
    fn equal_instants_keep_the_id_tiebreak_and_invalid_timestamps_fail_closed() {
        let ordered = recent_sessions(
            vec![
                session("one", "2026-09-09T00:00:00Z"),
                session("two", "2026-09-09T02:00:00.000+02:00"),
            ],
            1,
        )
        .unwrap();
        assert_eq!(ordered[0].id, "two");
        assert!(recent_sessions(vec![session("invalid", "not a timestamp")], 1).is_err());
    }
}
