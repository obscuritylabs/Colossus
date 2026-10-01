use colossus_api_proto::v1alpha1 as proto;

pub(super) fn summary_to_proto(
    value: colossus_api::ProcessSessionSummary,
) -> proto::ProcessSession {
    proto::ProcessSession {
        id: value.id,
        session_id: value.session_id,
        run_id: value.run_id,
        subagent_id: value.subagent_id,
        command: value.command,
        cwd: value.cwd,
        created_at_ms: value.created_at_ms,
        deadline_ms: value.deadline_ms,
        exit_code: value.exit_code,
        reason: value.reason,
        truncated: value.truncated,
        output_sequence: value.output_sequence,
        owner_id: value.owner.id,
        lifetime: match value.lifetime {
            colossus_api::ProcessLifetime::Run => "run",
            colossus_api::ProcessLifetime::Workspace => "workspace",
        }
        .into(),
        status: match value.status {
            colossus_api::ProcessSessionStatus::Starting => "starting",
            colossus_api::ProcessSessionStatus::Running => "running",
            colossus_api::ProcessSessionStatus::Stopping => "stopping",
            colossus_api::ProcessSessionStatus::Exited => "exited",
            colossus_api::ProcessSessionStatus::Stopped => "stopped",
            colossus_api::ProcessSessionStatus::TimedOut => "timed_out",
            colossus_api::ProcessSessionStatus::Failed => "failed",
            colossus_api::ProcessSessionStatus::Interrupted => "interrupted",
            colossus_api::ProcessSessionStatus::OutcomeUnknown => "outcome_unknown",
        }
        .into(),
    }
}
pub(super) fn snapshot_to_proto(
    value: colossus_api::ProcessSessionSnapshot,
) -> proto::ProcessSessionSnapshot {
    proto::ProcessSessionSnapshot {
        session: Some(summary_to_proto(value.session)),
        chunks: value
            .chunks
            .into_iter()
            .map(|chunk| proto::ProcessOutputChunk {
                sequence: chunk.sequence,
                stdout: chunk.stdout,
                stderr: chunk.stderr,
            })
            .collect(),
        next_sequence: value.next_sequence,
        gap: value.gap,
    }
}
