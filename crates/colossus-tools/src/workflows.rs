use super::*;

pub(super) fn workflow_specs() -> Vec<ToolSpec> {
    let id = json!({"type": "string", "minLength": 1, "maxLength": 128});
    let page = json!({"after": id, "limit": {"type": "integer", "minimum": 1, "maximum": 100}});
    let mut specs: Vec<_> = [
        ("workflow.definition.list", "workflow.definition.read", "List registered workflow identities and exact definition hashes in this Workspace.", page.clone(), vec![]),
        ("workflow.definition.get", "workflow.definition.read", "Inspect one registered workflow and its strict input schema before scheduling.", json!({"workflow_id": id}), vec!["workflow_id"]),
        ("workflow.schedule.list", "workflow.schedule.list", "List this application's schedules, with metadata-only legacy records. Lists omit input snapshots.", page, vec![]),
        ("workflow.schedule.get", "workflow.schedule.get", "Inspect a schedule's canonical inputs/task instructions, recurrence, origin, last dispatch, last independent run ID, and revision token.", json!({"schedule_id": id}), vec!["schedule_id"]),
        ("workflow.schedule.create", "workflow.schedule.create", "Request review of an immutable calendar or elapsed-interval workflow schedule, or a plain-language task, including initially paused schedules. Future runs require current policy and a running worker; schedules do not wake sleeping Workspaces. One due boundary always queues; multiple boundaries use fire_once or skip. Preserve the same retry key and exact intent when reconciling an uncertain result.", schedule_properties(), vec!["schedule_id", "workflow_id", "expected_hash", "inputs", "cadence_seconds", "starts_at", "misfire_policy", "enabled", "idempotency_key"]),
        ("workflow.schedule.set_enabled", "workflow.schedule.set_enabled", "Request authorized future-tick control at the exact schedule revision. Pause does not cancel queued/running work; enable preserves the boundary and reconciles missed occurrences. A stale token needs fresh inspection and review.", json!({"schedule_id": id, "enabled": {"type": "boolean"}, "etag": {"type": "string", "pattern": "^[a-f0-9]{64}$"}}), vec!["schedule_id", "enabled", "etag"]),
        ("workflow.schedule.delete", "workflow.schedule.delete", "Delete an exact caller-owned schedule revision from active discovery and future ticking. Existing queued/running runs and retained history remain. Inspect the exact etag before review; reconcile uncertain outcomes without allocating a replacement or changing the reviewed revision.", json!({"schedule_id": id, "etag": {"type": "string", "pattern": "^[a-f0-9]{64}$"}}), vec!["schedule_id", "etag"]),
    ].into_iter().map(|(name, action, description, properties, required)| {
        let mut input_schema = json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false});
        if name == "workflow.schedule.create" {
            input_schema["allOf"] = json!([
                {"oneOf": [
                    {"properties": {"task": {"type":"null"}, "workflow_id": {"minLength":1}, "expected_hash": {"pattern":"^[a-f0-9]{64}$"}}},
                    {"required":["task"], "properties": {"task": {"type":"object"}, "workflow_id": {"const":""}, "expected_hash":{"const":""}, "inputs":{"const":{}}}}
                ]},
                {"oneOf": [
                    {"properties": {"calendar": {"type":"null"}, "cadence_seconds":{"minimum":60}}},
                    {"required":["calendar"], "properties": {"calendar":{"type":"object"}, "cadence_seconds":{"const":0}}}
                ]}
            ]);
        }
        ToolSpec { name: name.into(), description: description.into(), input_schema, effect_action: Some(action.into()), capability: Some(action.into()), max_output_bytes: 1024 * 1024 }
    }).collect();
    specs.push(task_schedule_spec());
    specs
}

fn schedule_properties() -> Value {
    let nullable_profile = json!({"type":["string","null"], "minLength":1, "maxLength":128});
    json!({
        "schedule_id": {"type":"string", "minLength":1, "maxLength":128, "pattern":"^[a-z0-9][a-z0-9.-]*$"},
        "workflow_id": {"type":"string", "maxLength":128}, "expected_hash": {"type":"string", "maxLength":64},
        "inputs": {"type":"object"}, "cadence_seconds": {"type":"integer", "minimum":0, "maximum":2678400},
        "calendar": {"type":["object","null"], "additionalProperties":false, "required":["timezone","time","weekdays"], "properties":{
            "timezone":{"type":"string","minLength":1,"maxLength":128}, "time":{"type":"string","pattern":"^([01][0-9]|2[0-3]):[0-5][0-9]$"},
            "weekdays":{"type":"array","maxItems":7,"uniqueItems":true,"items":{"type":"integer","minimum":1,"maximum":7}}
        }},
        "task": {"type":["object","null"],"additionalProperties":false,"required":["name","instructions"],"properties":{
            "name":{"type":"string","minLength":1,"maxLength":128}, "instructions":{"type":"string","minLength":1,"maxLength":65536},
            "tools":{"type":"array","maxItems":128,"items":{"type":"string","minLength":1,"maxLength":128}},
            "options":{"type":"object","additionalProperties":false,"properties":{"model_profile":nullable_profile, "reasoning_effort":{"type":["string","null"],"enum":[null,"none","minimal","low","medium","high","xhigh","max","ultra"]}}}
        }},
        "starts_at":{"type":"string","minLength":1,"maxLength":64}, "misfire_policy":{"type":"string","enum":["fire_once","skip"]},
        "enabled":{"type":"boolean"}, "idempotency_key":{"type":"string","minLength":1,"maxLength":128}
    })
}

fn task_schedule_spec() -> ToolSpec {
    let properties = schedule_properties();
    let fields = [
        "schedule_id",
        "task",
        "calendar",
        "starts_at",
        "misfire_policy",
        "enabled",
        "idempotency_key",
    ];
    let mut task_properties = serde_json::Map::new();
    for field in fields {
        task_properties.insert(field.into(), properties[field].clone());
    }
    for field in ["task", "calendar"] {
        task_properties[field]["type"] = json!("object");
    }
    ToolSpec {
        name: "workflow.task.schedule".into(),
        description: "Schedule a plain-language agent task with daily or selected-weekday calendar timing. No registered workflow, definition hash, or JSON inputs are needed. Specify an exact first UTC occurrence matching the IANA timezone/local time, allowed task tools, configured model preferences, and one durable retry key. Uses normal schedule creation policy and review, even when paused. Inspect existing schedules first; preserve the exact request on uncertain retry and confirm the stored schedule. Requires a running worker; does not wake a sleeping computer.".into(),
        input_schema: json!({"type":"object", "properties":task_properties, "required":fields, "additionalProperties":false}),
        effect_action: Some("workflow.schedule.create".into()),
        capability: Some("workflow.schedule.create".into()),
        max_output_bytes: 1024 * 1024,
    }
}
