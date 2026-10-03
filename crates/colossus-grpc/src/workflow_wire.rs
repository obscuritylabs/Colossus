//! Bounded workflow value translations shared with the authenticated Rust client.
use colossus_api::{
    Actor, ActorType, RegisteredWorkflow, WorkflowOrigin, WorkflowRunSnapshot, WorkflowSchedule,
    WorkflowScheduleDispatchStatus as Dispatch, WorkflowScheduleMisfirePolicy as Misfire,
    WorkflowScheduleSnapshot, WorkflowStatus,
};
use colossus_api_proto::v1alpha1 as proto;
use prost_types::{Struct, Timestamp, Value, value::Kind};
use serde_json::{Map, Value as Json};
use tonic::Status;

fn invalid() -> Status {
    Status::invalid_argument("invalid bounded workflow value")
}

/// Convert a bounded JSON object to protobuf without accepting non-finite numbers.
pub fn object(value: &Json) -> Result<Option<Struct>, Status> {
    if value.is_null() {
        return Ok(None);
    }
    if serde_json::to_vec(value).map_err(|_| invalid())?.len() > 256 * 1024 {
        return Err(invalid());
    }
    let Kind::StructValue(value) = to_value(value, 0)?.kind.ok_or_else(invalid)? else {
        return Err(invalid());
    };
    Ok(Some(value))
}
fn to_value(value: &Json, depth: usize) -> Result<Value, Status> {
    if depth > 32 {
        return Err(invalid());
    }
    let kind = match value {
        Json::Null => Kind::NullValue(0),
        Json::Bool(value) => Kind::BoolValue(*value),
        Json::Number(value) => {
            // Struct uses IEEE-754 doubles. Reject integers that would silently
            // change the reviewed input at the transport boundary.
            if value
                .as_i64()
                .is_some_and(|n| !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&n))
                || value.as_u64().is_some_and(|n| n > 9_007_199_254_740_991)
            {
                return Err(invalid());
            }
            Kind::NumberValue(value.as_f64().ok_or_else(invalid)?)
        }
        Json::String(value) => Kind::StringValue(value.clone()),
        Json::Array(values) => Kind::ListValue(prost_types::ListValue {
            values: values
                .iter()
                .map(|value| to_value(value, depth + 1))
                .collect::<Result<_, _>>()?,
        }),
        Json::Object(values) => Kind::StructValue(Struct {
            fields: values
                .iter()
                .map(|(key, value)| Ok((key.clone(), to_value(value, depth + 1)?)))
                .collect::<Result<_, Status>>()?,
        }),
    };
    Ok(Value { kind: Some(kind) })
}
/// Decode a bounded protobuf object with strict value and depth checks.
pub fn json(value: Option<Struct>) -> Result<Json, Status> {
    let value = from_value(
        Value {
            kind: Some(Kind::StructValue(value.unwrap_or_default())),
        },
        0,
    )?;
    if serde_json::to_vec(&value).map_err(|_| invalid())?.len() > 256 * 1024 {
        return Err(invalid());
    }
    Ok(value)
}
fn from_value(value: Value, depth: usize) -> Result<Json, Status> {
    if depth > 32 {
        return Err(invalid());
    }
    Ok(match value.kind.ok_or_else(invalid)? {
        Kind::NullValue(0) => Json::Null,
        Kind::NullValue(_) => return Err(invalid()),
        Kind::BoolValue(value) => Json::Bool(value),
        Kind::StringValue(value) => Json::String(value),
        Kind::NumberValue(value) => {
            Json::Number(serde_json::Number::from_f64(value).ok_or_else(invalid)?)
        }
        Kind::ListValue(value) => Json::Array(
            value
                .values
                .into_iter()
                .map(|value| from_value(value, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        Kind::StructValue(value) => Json::Object(
            value
                .fields
                .into_iter()
                .map(|(key, value)| Ok((key, from_value(value, depth + 1)?)))
                .collect::<Result<Map<_, _>, Status>>()?,
        ),
    })
}
/// Validate and format a required protobuf timestamp as a canonical UTC instant.
pub fn instant(value: Option<Timestamp>) -> Result<String, Status> {
    let value = value.ok_or_else(invalid)?;
    if !(-62_135_596_800..=253_402_300_799).contains(&value.seconds)
        || !(0..1_000_000_000).contains(&value.nanos)
    {
        return Err(invalid());
    }
    Ok(value.to_string())
}
fn timestamp(value: &str) -> Result<Timestamp, Status> {
    let result: Timestamp = value.parse().map_err(|_| invalid())?;
    instant(Some(result))?;
    Ok(result)
}
fn identity(value: &str) -> Result<(), Status> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn hash(value: &str) -> Result<(), Status> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
/// Encode released registered-definition metadata.
pub fn definition(value: RegisteredWorkflow) -> Result<proto::WorkflowSummary, Status> {
    Ok(proto::WorkflowSummary {
        workflow_id: value.workflow_id,
        name: value.name,
        version: value.version,
        description: value.description,
        enabled: value.scheduling_eligible,
        etag: value.workflow_hash.clone(),
        definition_hash: value.workflow_hash,
        input_schema: object(&value.input_schema)?,
        unavailable_reason: value.unavailable_reason,
        created_at: None,
        updated_at: None,
    })
}
/// Decode and validate registered-definition metadata from a remote target.
pub fn decode_definition(value: proto::WorkflowSummary) -> Result<RegisteredWorkflow, Status> {
    identity(&value.workflow_id)?;
    identity(&value.name)?;
    identity(&value.version)?;
    hash(&value.definition_hash)?;
    if value.workflow_id != format!("{}:{}", value.name, value.version)
        || value.description.len() > 4096
        || value
            .unavailable_reason
            .as_ref()
            .is_some_and(|value| value.len() > 4096)
    {
        return Err(invalid());
    }
    Ok(RegisteredWorkflow {
        workflow_id: value.workflow_id,
        name: value.name,
        version: value.version,
        workflow_hash: value.definition_hash,
        description: value.description,
        input_schema: if value.input_schema.is_some() {
            json(value.input_schema)?
        } else {
            Json::Null
        },
        scheduling_eligible: value.enabled,
        unavailable_reason: value.unavailable_reason,
    })
}
/// Decode an explicitly selected misfire policy; unspecified is never a default.
pub fn misfire(value: i32) -> Result<Misfire, Status> {
    match proto::ScheduleMisfirePolicy::try_from(value) {
        Ok(proto::ScheduleMisfirePolicy::FireOnce) => Ok(Misfire::FireOnce),
        Ok(proto::ScheduleMisfirePolicy::Skip) => Ok(Misfire::Skip),
        _ => Err(invalid()),
    }
}
/// Encode a fixed-cadence misfire policy.
pub fn encode_misfire(value: Misfire) -> i32 {
    match value {
        Misfire::FireOnce => proto::ScheduleMisfirePolicy::FireOnce as i32,
        Misfire::Skip => proto::ScheduleMisfirePolicy::Skip as i32,
    }
}
/// Encode authorized schedule detail or metadata-only summary.
pub fn schedule(value: WorkflowScheduleSnapshot) -> Result<proto::WorkflowSchedule, Status> {
    let record = value.record;
    Ok(proto::WorkflowSchedule {
        schedule_id: record.schedule_id,
        workflow_id: format!("{}:{}", record.workflow_name, record.workflow_version),
        definition_hash: record.workflow_hash,
        input: object(&record.inputs)?,
        cadence_seconds: record.cadence_seconds,
        misfire_policy: encode_misfire(record.misfire_policy),
        enabled: record.enabled,
        starts_at: Some(timestamp(&record.starts_at)?),
        next_fire_at: Some(timestamp(&record.next_fire_at)?),
        last_scheduled_at: record
            .last_scheduled_at
            .as_deref()
            .map(timestamp)
            .transpose()?,
        last_workflow_run_id: record.last_run_id,
        blocked_reason: record.blocked_reason,
        created_at: Some(timestamp(&record.created_at)?),
        updated_at: Some(timestamp(&record.updated_at)?),
        etag: value.etag,
        origin: value.origin.map(|origin| proto::WorkflowScheduleOrigin {
            application_id: origin.owner.id,
            session_id: origin.session_id,
            run_id: origin.run_id,
        }),
        controllable: value.controllable,
        last_dispatch: match value.last_dispatch {
            None => proto::ScheduleDispatchStatus::Unspecified,
            Some(Dispatch::Queued) => proto::ScheduleDispatchStatus::Queued,
            Some(Dispatch::Skipped) => proto::ScheduleDispatchStatus::Skipped,
            Some(Dispatch::Blocked) => proto::ScheduleDispatchStatus::Blocked,
        } as i32,
    })
}
/// Decode a schedule while rejecting unknown states and fabricated legacy ownership.
pub fn decode_schedule(value: proto::WorkflowSchedule) -> Result<WorkflowScheduleSnapshot, Status> {
    identity(&value.schedule_id)?;
    identity(&value.workflow_id)?;
    hash(&value.definition_hash)?;
    hash(&value.etag)?;
    let (name, version) = value.workflow_id.split_once(':').ok_or_else(invalid)?;
    if !(60..=2_678_400).contains(&value.cadence_seconds)
        || value.controllable != value.origin.is_some()
        || (!value.controllable && (value.input.is_some() || value.last_workflow_run_id.is_some()))
        || value
            .blocked_reason
            .as_ref()
            .is_some_and(|value| value.len() > 4096)
    {
        return Err(invalid());
    }
    let origin = value
        .origin
        .map(|origin| {
            identity(&origin.application_id)?;
            if let Some(id) = &origin.session_id {
                identity(id)?;
            }
            if let Some(id) = &origin.run_id {
                identity(id)?;
            }
            Ok::<_, Status>(WorkflowOrigin {
                owner: Actor {
                    actor_type: ActorType::Application,
                    id: origin.application_id,
                },
                session_id: origin.session_id,
                run_id: origin.run_id,
            })
        })
        .transpose()?;
    let last_dispatch = match proto::ScheduleDispatchStatus::try_from(value.last_dispatch) {
        Ok(proto::ScheduleDispatchStatus::Unspecified) => None,
        Ok(proto::ScheduleDispatchStatus::Queued) => Some(Dispatch::Queued),
        Ok(proto::ScheduleDispatchStatus::Skipped) => Some(Dispatch::Skipped),
        Ok(proto::ScheduleDispatchStatus::Blocked) => Some(Dispatch::Blocked),
        _ => return Err(invalid()),
    };
    Ok(WorkflowScheduleSnapshot {
        record: WorkflowSchedule {
            schedule_id: value.schedule_id,
            workflow_name: name.into(),
            workflow_version: version.into(),
            workflow_hash: value.definition_hash,
            inputs: if value.input.is_some() {
                json(value.input)?
            } else {
                Json::Null
            },
            cadence_seconds: value.cadence_seconds,
            misfire_policy: misfire(value.misfire_policy)?,
            enabled: value.enabled,
            starts_at: instant(value.starts_at)?,
            next_fire_at: instant(value.next_fire_at)?,
            last_scheduled_at: value
                .last_scheduled_at
                .map(|value| instant(Some(value)))
                .transpose()?,
            last_run_id: value.last_workflow_run_id,
            blocked_reason: value.blocked_reason,
            created_at: instant(value.created_at)?,
            updated_at: instant(value.updated_at)?,
        },
        origin,
        etag: value.etag,
        controllable: value.controllable,
        last_dispatch,
    })
}
/// Encode safe independent workflow-run state.
pub fn run(value: WorkflowRunSnapshot) -> Result<proto::WorkflowRun, Status> {
    Ok(proto::WorkflowRun {
        workflow_run_id: value.run_id,
        workflow_id: value.workflow_id,
        status: match value.status {
            WorkflowStatus::Queued => proto::WorkflowRunStatus::Queued,
            WorkflowStatus::Running => proto::WorkflowRunStatus::Running,
            WorkflowStatus::Waiting => proto::WorkflowRunStatus::Waiting,
            WorkflowStatus::Completed => proto::WorkflowRunStatus::Completed,
            WorkflowStatus::Failed => proto::WorkflowRunStatus::Failed,
            WorkflowStatus::Cancelled => proto::WorkflowRunStatus::Cancelled,
            WorkflowStatus::Interrupted => proto::WorkflowRunStatus::Interrupted,
        } as i32,
        created_at: Some(timestamp(&value.created_at)?),
        updated_at: Some(timestamp(&value.updated_at)?),
        last_sequence: value.last_sequence,
        failure_reason: value.failure_reason,
        waiting_reason: value.waiting_reason,
        definition_hash: value.workflow_hash,
    })
}
/// Decode safe workflow-run state; unknown states fail closed.
pub fn decode_run(value: proto::WorkflowRun) -> Result<WorkflowRunSnapshot, Status> {
    identity(&value.workflow_run_id)?;
    identity(&value.workflow_id)?;
    hash(&value.definition_hash)?;
    if value.last_sequence == 0
        || value
            .failure_reason
            .as_ref()
            .is_some_and(|value| value.len() > 4096)
        || value
            .waiting_reason
            .as_ref()
            .is_some_and(|value| value.len() > 4096)
    {
        return Err(invalid());
    }
    let status = match proto::WorkflowRunStatus::try_from(value.status) {
        Ok(proto::WorkflowRunStatus::Queued) => WorkflowStatus::Queued,
        Ok(proto::WorkflowRunStatus::Running) => WorkflowStatus::Running,
        Ok(proto::WorkflowRunStatus::Waiting) => WorkflowStatus::Waiting,
        Ok(proto::WorkflowRunStatus::Completed) => WorkflowStatus::Completed,
        Ok(proto::WorkflowRunStatus::Failed) => WorkflowStatus::Failed,
        Ok(proto::WorkflowRunStatus::Cancelled) => WorkflowStatus::Cancelled,
        Ok(proto::WorkflowRunStatus::Interrupted) => WorkflowStatus::Interrupted,
        _ => return Err(invalid()),
    };
    Ok(WorkflowRunSnapshot {
        run_id: value.workflow_run_id,
        workflow_id: value.workflow_id,
        workflow_hash: value.definition_hash,
        status,
        created_at: instant(value.created_at)?,
        updated_at: instant(value.updated_at)?,
        last_sequence: value.last_sequence,
        failure_reason: value.failure_reason,
        waiting_reason: value.waiting_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewed_integers_do_not_round_at_the_protobuf_boundary() {
        assert!(object(&serde_json::json!({"counter": 9_007_199_254_740_992_u64})).is_err());
        assert!(object(&serde_json::json!({"counter": -9_007_199_254_740_992_i64})).is_err());
        let value =
            json(object(&serde_json::json!({"counter": 9_007_199_254_740_991_u64})).unwrap())
                .unwrap();
        assert_eq!(value["counter"].as_f64(), Some(9_007_199_254_740_991.0));
    }

    #[test]
    fn malformed_values_states_and_timestamps_fail_closed() {
        assert!(
            instant(Some(Timestamp {
                seconds: 0,
                nanos: -1
            }))
            .is_err()
        );
        assert!(
            instant(Some(Timestamp {
                seconds: 253_402_300_800,
                nanos: 0
            }))
            .is_err()
        );
        assert!(misfire(0).is_err());
        assert!(misfire(999).is_err());
        assert!(
            json(Some(Struct {
                fields: [("secret".into(), Value { kind: None })].into()
            }))
            .is_err()
        );
        assert!(
            json(Some(Struct {
                fields: [(
                    "number".into(),
                    Value {
                        kind: Some(Kind::NumberValue(f64::NAN))
                    }
                )]
                .into()
            }))
            .is_err()
        );
    }
}
