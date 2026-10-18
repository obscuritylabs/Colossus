use super::service::{ComputerRun, RuntimeComputerTools, denied};
use crate::prelude::*;
use colossus_contracts::ComputerSessionId;
use colossus_ports::ComputerCommand;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ComputerInvocation {
    EmptyStatus {},
    Command { command: ComputerCommand },
}

impl ComputerInvocation {
    pub(super) fn from_call(
        call: &ToolCall,
        context: &ExecutionContext,
        service: &RuntimeComputerTools,
        run: &ComputerRun,
    ) -> Result<Self, ToolError> {
        let action = call.name.strip_prefix("computer.").ok_or_else(denied)?;
        let mut arguments = call
            .arguments
            .as_object()
            .cloned()
            .ok_or_else(|| invalid(call))?;
        if action == "status" && !arguments.contains_key("session_id") {
            let state = run.state.lock().map_err(|_| denied())?;
            let Some(session) = &state.session else {
                return Ok(Self::EmptyStatus {});
            };
            arguments.insert("session_id".into(), json!(session.session_id));
        }
        let command = match action {
            "windows" => {
                arguments
                    .entry("max_windows")
                    .or_insert(json!(service.limits.max_windows));
                arguments.insert("kind".into(), json!("windows"));
                decode(Value::Object(arguments), call)?
            }
            "open" => {
                arguments.insert("kind".into(), json!("open"));
                arguments.insert("limits".into(), json!(service.limits));
                decode(Value::Object(arguments), call)?
            }
            "snapshot" => {
                arguments
                    .entry("max_nodes")
                    .or_insert(json!(service.limits.max_snapshot_nodes));
                arguments.insert("kind".into(), json!("snapshot"));
                decode(Value::Object(arguments), call)?
            }
            "screenshot" => {
                arguments.insert("kind".into(), json!("screenshot"));
                let max_bytes = context.image_input_budget.as_ref().map_or(
                    service.limits.max_screenshot_bytes,
                    |budget| {
                        u32::try_from(budget.remaining_bytes)
                            .unwrap_or(u32::MAX)
                            .min(service.limits.max_screenshot_bytes)
                    },
                );
                arguments.insert("max_bytes".into(), json!(max_bytes));
                decode(Value::Object(arguments), call)?
            }
            "status" | "close" => {
                arguments.insert("kind".into(), json!(action));
                decode(Value::Object(arguments), call)?
            }
            "focus" | "move" | "click" | "drag" | "scroll" | "type" | "press" => {
                if action == "drag" {
                    arguments.entry("steps").or_insert(json!(16));
                }
                let session_id = arguments
                    .remove("session_id")
                    .ok_or_else(|| invalid(call))?;
                let generation = arguments
                    .remove("control_generation")
                    .ok_or_else(|| invalid(call))?;
                let observation_id = arguments
                    .remove("observation_id")
                    .ok_or_else(|| invalid(call))?;
                arguments.insert("kind".into(), json!(action));
                decode(
                    json!({"kind":"input","session_id":session_id,"control_generation":generation,
                    "observation_id":observation_id,"input":arguments}),
                    call,
                )?
            }
            _ => return Err(invalid(call)),
        };
        command.validate().map_err(|_| invalid(call))?;
        Ok(Self::Command { command })
    }

    pub(super) fn action(&self) -> &'static str {
        use colossus_contracts::ComputerInput as Input;
        match self {
            Self::EmptyStatus {} => "computer.status",
            Self::Command { command } => match command {
                ComputerCommand::Windows { .. } => "computer.windows",
                ComputerCommand::Open { .. } => "computer.open",
                ComputerCommand::Status { .. } => "computer.status",
                ComputerCommand::Snapshot { .. } => "computer.snapshot",
                ComputerCommand::Screenshot { .. } => "computer.screenshot",
                ComputerCommand::Close { .. } => "computer.close",
                ComputerCommand::Input { input, .. } => match input {
                    Input::Focus { .. } => "computer.focus",
                    Input::Move { .. } => "computer.move",
                    Input::Click { .. } => "computer.click",
                    Input::Drag { .. } => "computer.drag",
                    Input::Scroll { .. } => "computer.scroll",
                    Input::Type { .. } => "computer.type",
                    Input::Press { .. } => "computer.press",
                },
            },
        }
    }

    pub(super) fn resource(&self) -> String {
        self.session_id()
            .map_or_else(|| "computer:windows".into(), |id| format!("computer:{id}"))
    }

    pub(super) fn session_id(&self) -> Option<&ComputerSessionId> {
        let Self::Command { command } = self else {
            return None;
        };
        match command {
            ComputerCommand::Windows { .. } | ComputerCommand::Open { .. } => None,
            ComputerCommand::Status { session_id }
            | ComputerCommand::Snapshot { session_id, .. }
            | ComputerCommand::Screenshot { session_id, .. }
            | ComputerCommand::Input { session_id, .. }
            | ComputerCommand::Close { session_id, .. } => Some(session_id),
        }
    }
}

fn decode(value: Value, call: &ToolCall) -> Result<ComputerCommand, ToolError> {
    serde_json::from_value(value).map_err(|_| invalid(call))
}

fn invalid(call: &ToolCall) -> ToolError {
    ToolError::InvalidArguments {
        tool: call.name.clone(),
        message: "invalid bounded computer arguments".into(),
    }
}
