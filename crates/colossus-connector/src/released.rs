use colossus_cloud_protocol::{CloudReply, encode};
use colossus_sdk::{CancelRunResponse, GetRunResponse, RunTerminal};

/// Cancellation of an already-terminal run can return its large released result.
/// The receipt confirms the exact run; snapshots own the explicit output-limit marker.
pub(crate) fn compact_cancellation(response: &mut CancelRunResponse) -> Result<(), &'static str> {
    let mut snapshot = GetRunResponse {
        run: response.run.clone(),
        pending_interactions: vec![],
    };
    compact_snapshot(&mut snapshot)?;
    response.run = snapshot.run;
    Ok(())
}

/// Keep lifecycle/identity metadata observable when released content exceeds the
/// cloud frame budget. Whole interactions are omitted, never partially approved.
/// The caller must durably mark the cloud projection incomplete.
pub(crate) fn compact_snapshot(snapshot: &mut GetRunResponse) -> Result<bool, &'static str> {
    let fits = |snapshot: &GetRunResponse| {
        encode(&CloudReply::Run {
            run: Box::new(snapshot.clone()),
        })
        .is_ok()
    };
    if fits(snapshot) {
        return Ok(false);
    }
    if let Some(terminal) = &mut snapshot.run.terminal {
        match terminal {
            RunTerminal::Result(result) => result.output.clear(),
            RunTerminal::Failure(failure) => {
                failure.message =
                    "Run details exceed the cloud frame budget; open this run locally.".into()
            }
            RunTerminal::Cancellation(cancellation) => {
                cancellation.message =
                    "Run details exceed the cloud frame budget; open this run locally.".into()
            }
        }
    }
    if !fits(snapshot) {
        snapshot.pending_interactions.clear();
    }
    if fits(snapshot) {
        Ok(true)
    } else {
        Err("run metadata exceeds cloud frame budget")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_cloud_protocol::MAX_PAYLOAD_BYTES;
    use colossus_sdk::{Run, RunMode, RunResult, RunStatus};

    #[test]
    fn oversized_terminal_output_keeps_identity_lifecycle_and_cursor() {
        let mut snapshot = GetRunResponse {
            run: Run {
                plugin_skill_ids: vec![],
                run_id: "run-1".into(),
                session_id: "session-1".into(),
                title: "Test task".into(),
                role: "primary".into(),
                mode: RunMode::Execute,
                status: RunStatus::Completed,
                created_at: "2026-10-05T00:00:00Z".into(),
                updated_at: "2026-10-05T00:00:01Z".into(),
                started_at: None,
                finished_at: None,
                last_sequence: 17,
                pending_interaction_count: 0,
                etag: "revision-17".into(),
                archived: false,
                terminal: Some(RunTerminal::Result(RunResult {
                    output: "x".repeat(MAX_PAYLOAD_BYTES),
                    plan_id: None,
                    plan_revision: None,
                    plan_status: None,
                    goal_id: None,
                    profile: "fixture".into(),
                    model_profile: "fixture".into(),
                    provider_profile: "fixture".into(),
                    model: "fixture".into(),
                    elapsed_seconds: 1.0,
                })),
            },
            pending_interactions: vec![],
        };
        let mut cancelled = CancelRunResponse {
            run: snapshot.run.clone(),
        };
        compact_cancellation(&mut cancelled).unwrap();
        assert_eq!(cancelled.run.run_id, "run-1");
        assert_eq!(cancelled.run.status, RunStatus::Completed);
        assert!(
            encode(&CloudReply::Cancelled {
                response: cancelled
            })
            .is_ok()
        );
        assert!(compact_snapshot(&mut snapshot).unwrap());
        assert_eq!(snapshot.run.run_id, "run-1");
        assert_eq!(snapshot.run.status, RunStatus::Completed);
        assert_eq!(snapshot.run.last_sequence, 17);
        assert_eq!(snapshot.run.etag, "revision-17");
        assert!(
            encode(&CloudReply::Run {
                run: Box::new(snapshot.clone())
            })
            .is_ok()
        );
        assert!(!compact_snapshot(&mut snapshot).unwrap());
    }
}
