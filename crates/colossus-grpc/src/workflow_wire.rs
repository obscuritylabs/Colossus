//! Bounded workflow value translations shared with the authenticated Rust client.
use colossus_api::{
    Actor, ActorType, RegisteredWorkflow, WorkflowLogic, WorkflowOrigin, WorkflowRunSnapshot,
    WorkflowSchedule, WorkflowScheduleDispatchStatus as Dispatch,
    WorkflowScheduleMisfirePolicy as Misfire, WorkflowScheduleSnapshot, WorkflowStatus,
    WorkflowStepState,
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
        logic: value
            .logic
            .as_ref()
            .map(|logic| {
                if !logic.within_bounds() {
                    return Err(invalid());
                }
                object(&serde_json::to_value(logic).map_err(|_| invalid())?)
            })
            .transpose()?
            .flatten(),
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
        logic: value
            .logic
            .map(|value| {
                let logic: WorkflowLogic =
                    serde_json::from_value(json(Some(value))?).map_err(|_| invalid())?;
                if !logic.within_bounds() {
                    return Err(invalid());
                }
                Ok(logic)
            })
            .transpose()?,
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
        calendar: encode_optional(&record.calendar)?,
        task: encode_optional(&record.task)?,
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
    if (if value.calendar.is_some() {
        value.cadence_seconds != 0
    } else {
        !(60..=2_678_400).contains(&value.cadence_seconds)
    }) || value.controllable != value.origin.is_some()
        || (!value.controllable
            && (value.input.is_some()
                || value.last_workflow_run_id.is_some()
                || value.task.is_some()))
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
            calendar: decode_calendar(value.calendar)?,
            task: decode_optional(value.task)?,
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
        result_json: value
            .result
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| invalid())?,
        step_states: object(&serde_json::json!({"steps": value.step_states}))?,
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
        result: value
            .result_json
            .map(|result| {
                if result.len() > 64 * 1024 {
                    return Err(invalid());
                }
                let value: Json = serde_json::from_str(&result).map_err(|_| invalid())?;
                if !value.is_object() {
                    return Err(invalid());
                }
                Ok(value)
            })
            .transpose()?,
        step_states: decode_step_states(value.step_states)?,
    })
}

fn decode_step_states(value: Option<Struct>) -> Result<Vec<WorkflowStepState>, Status> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct States {
        steps: Vec<WorkflowStepState>,
    }
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    // Struct numbers are doubles. Normalize only this bounded integral field;
    // never change arbitrary reviewed input/schema numbers at the transport boundary.
    let mut decoded = json(Some(value))?;
    let steps = decoded
        .get_mut("steps")
        .and_then(Json::as_array_mut)
        .ok_or_else(invalid)?;
    if steps.len() > 512 {
        return Err(invalid());
    }
    for step in steps {
        let count = step
            .get("completed_executions")
            .and_then(Json::as_f64)
            .ok_or_else(invalid)?;
        if !(0.0..=10000.0).contains(&count) || count.fract() != 0.0 {
            return Err(invalid());
        }
        step["completed_executions"] =
            Json::from(count.to_string().parse::<u32>().map_err(|_| invalid())?);
    }
    let states: States = serde_json::from_value(decoded).map_err(|_| invalid())?;
    if states.steps.len() > 512 {
        return Err(invalid());
    }
    let mut ids = std::collections::BTreeSet::new();
    for step in &states.steps {
        identity(&step.step_id)?;
        if step.step_id.len() > 128
            || step.completed_executions > 10000
            || !ids.insert(&step.step_id)
        {
            return Err(invalid());
        }
    }
    Ok(states.steps)
}

/// Encode an optional strict bounded workflow resource as a protobuf object.
pub fn encode_optional<T: serde::Serialize>(
    value: &Option<T>,
) -> Result<Option<prost_types::Struct>, tonic::Status> {
    value
        .as_ref()
        .map(|value| object(&serde_json::to_value(value).map_err(|_| invalid())?))
        .transpose()
        .map(Option::flatten)
}
/// Decode an optional object through its strict typed contract.
pub fn decode_optional<T: serde::de::DeserializeOwned>(
    value: Option<prost_types::Struct>,
) -> Result<Option<T>, tonic::Status> {
    value
        .map(|value| serde_json::from_value(json(Some(value))?).map_err(|_| invalid()))
        .transpose()
}

/// Decode ISO weekday integers without silently rounding Struct's doubles.
pub fn decode_calendar(
    value: Option<Struct>,
) -> Result<Option<colossus_api::WorkflowCalendar>, Status> {
    let Some(value) = value else {
        return Ok(None);
    };
    let mut value = json(Some(value))?;
    if let Some(days) = value.get_mut("weekdays").and_then(Json::as_array_mut) {
        if days.len() > 7 {
            return Err(invalid());
        }
        for day in days {
            let number = day.as_f64().ok_or_else(invalid)?;
            if !(1.0..=7.0).contains(&number) || number.fract() != 0.0 {
                return Err(invalid());
            }
            // Seven bounded exact values, avoiding unchecked float-to-integer conversion.
            *day = (1_u8..=7)
                .find(|value| f64::from(*value) == number)
                .map(Json::from)
                .ok_or_else(invalid)?;
        }
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_weekdays_reject_fractional_transport_values() {
        let value =
            serde_json::json!({"timezone":"America/New_York","time":"09:00","weekdays":[1,5]});
        assert_eq!(
            decode_calendar(object(&value).unwrap())
                .unwrap()
                .unwrap()
                .weekdays,
            vec![1, 5]
        );
        for day in [0.0, 1.5, 8.0] {
            let mut invalid = value.clone();
            invalid["weekdays"] = serde_json::json!([day]);
            assert!(decode_calendar(object(&invalid).unwrap()).is_err());
        }
    }

    #[test]
    fn final_result_roundtrips_exact_integers_and_rejects_non_objects() {
        let snapshot = WorkflowRunSnapshot {
            run_id: "run-1".into(),
            workflow_id: "health:1.0.0".into(),
            workflow_hash: "a".repeat(64),
            status: WorkflowStatus::Completed,
            created_at: "2026-10-04T12:00:00Z".into(),
            updated_at: "2026-10-04T12:00:01Z".into(),
            last_sequence: 3,
            failure_reason: None,
            waiting_reason: None,
            step_states: vec![],
            result: Some(serde_json::json!({"exact":u64::MAX})),
        };
        let wire = run(snapshot.clone()).unwrap();
        assert_eq!(decode_run(wire.clone()).unwrap().result, snapshot.result);
        let mut invalid = wire;
        invalid.result_json = Some("[]".into());
        assert!(decode_run(invalid).is_err());
    }

    #[test]
    fn recorded_counts_roundtrip_without_accepting_fractional_or_duplicate_states() {
        let value = serde_json::json!({"steps": [{"step_id": "loop-item", "status": "completed", "completed_executions": 4}]});
        let decoded = decode_step_states(object(&value).unwrap()).unwrap();
        assert_eq!(decoded[0].completed_executions, 4);
        for count in [
            serde_json::json!(-1),
            serde_json::json!(0.5),
            serde_json::json!(10001),
        ] {
            let mut invalid = value.clone();
            invalid["steps"][0]["completed_executions"] = count;
            assert!(decode_step_states(object(&invalid).unwrap()).is_err());
        }
        let mut duplicate = value.clone();
        duplicate["steps"]
            .as_array_mut()
            .unwrap()
            .push(value["steps"][0].clone());
        assert!(decode_step_states(object(&duplicate).unwrap()).is_err());
        assert_eq!(decode_step_states(None).unwrap(), Vec::new());
    }

    #[test]
    fn logic_roundtrip_rejects_duplicate_ids_unknown_payloads_and_malformed_branches() {
        let value = serde_json::json!({"steps": [{"id": "result", "kind": "emit", "summary": "Emit a workflow value", "branches": []}], "compensation": []});
        let decoded: WorkflowLogic =
            serde_json::from_value(json(object(&value).unwrap()).unwrap()).unwrap();
        assert!(decoded.within_bounds());
        let mut duplicate = decoded.clone();
        duplicate.steps.push(decoded.steps[0].clone());
        assert!(!duplicate.within_bounds());
        let mut malformed = decoded.clone();
        malformed.steps[0].kind = colossus_api::WorkflowLogicKind::Condition;
        assert!(!malformed.within_bounds());
        let mut unexpected = value;
        unexpected["steps"][0]["arguments"] = serde_json::json!({"secret": "withheld"});
        assert!(serde_json::from_value::<WorkflowLogic>(unexpected).is_err());

        let mut nested = decoded.clone();
        for depth in 0..8 {
            nested.steps = vec![colossus_api::WorkflowLogicStep {
                id: format!("route-{depth}"),
                kind: colossus_api::WorkflowLogicKind::Condition,
                summary: "/inputs/production == true".into(),
                branches: vec![
                    colossus_api::WorkflowLogicBranch {
                        label: "True".into(),
                        steps: nested.steps,
                    },
                    colossus_api::WorkflowLogicBranch {
                        label: "False".into(),
                        steps: Vec::new(),
                    },
                ],
            }];
            if depth < 7 {
                assert!(nested.within_bounds());
                let value = serde_json::to_value(&nested).unwrap();
                assert_eq!(json(object(&value).unwrap()).unwrap(), value);
            } else {
                assert!(!nested.within_bounds());
            }
        }
        let mut oversized = decoded.clone();
        oversized.steps = (0..80)
            .map(|index| colossus_api::WorkflowLogicStep {
                id: format!("step-{index}"),
                summary: "x".repeat(4096),
                ..decoded.steps[0].clone()
            })
            .collect();
        assert!(!oversized.within_bounds());
    }

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
