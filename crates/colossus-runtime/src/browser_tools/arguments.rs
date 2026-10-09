use crate::prelude::*;
use colossus_contracts::{
    BrowserAction, BrowserElementRef, BrowserOpenOptions, BrowserSessionId, BrowserTarget,
};
use serde::de::DeserializeOwned;

/// Prepared content is hashed into the normal one-use effect permit.
#[derive(Clone, Serialize, serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum BrowserInvocation {
    Open {
        options: BrowserOpenOptions,
    },
    Status {
        session_id: BrowserSessionId,
        tabs_only: bool,
    },
    Close {
        session_id: BrowserSessionId,
        control_generation: u64,
    },
    Action {
        session_id: BrowserSessionId,
        control_generation: u64,
        target: Option<BrowserTarget>,
        action: BrowserAction,
    },
}

impl BrowserInvocation {
    pub(super) fn from_call(call: &ToolCall) -> Result<Self, ToolError> {
        if call.name == "browser.open" {
            return Ok(Self::Open {
                options: decode(call, &call.arguments)?,
            });
        }
        let session_id = field(call, "session_id")?;
        if matches!(call.name.as_str(), "browser.status" | "browser.tabs") {
            return Ok(Self::Status {
                session_id,
                tabs_only: call.name == "browser.tabs",
            });
        }
        let control_generation = field(call, "control_generation")?;
        if call.name == "browser.close" {
            return Ok(Self::Close {
                session_id,
                control_generation,
            });
        }
        let target = if matches!(
            call.name.as_str(),
            "browser.tab.open" | "browser.tab.select" | "browser.tab.close"
        ) {
            None
        } else {
            Some(BrowserTarget {
                tab_id: field(call, "tab_id")?,
                document_id: field(call, "document_id")?,
            })
        };
        let element = || -> Result<BrowserElementRef, ToolError> {
            Ok(BrowserElementRef {
                document_id: field(call, "document_id")?,
                snapshot_id: field(call, "snapshot_id")?,
                element_id: field(call, "element_id")?,
            })
        };
        let action = match call.name.as_str() {
            "browser.navigate" => BrowserAction::Navigate {
                url: field(call, "url")?,
            },
            "browser.snapshot" => BrowserAction::Snapshot {
                max_nodes: field(call, "max_nodes")?,
            },
            "browser.click" => BrowserAction::Click {
                element: element()?,
            },
            "browser.fill" => {
                let text: String = field(call, "text")?;
                if text.len() > 8192 {
                    return Err(invalid(call));
                }
                BrowserAction::Fill {
                    element: element()?,
                    text,
                }
            }
            "browser.select" => {
                let values: Vec<String> = field(call, "values")?;
                if values.is_empty()
                    || values.len() > 32
                    || values.iter().any(|value| value.len() > 1024)
                {
                    return Err(invalid(call));
                }
                BrowserAction::Select {
                    element: element()?,
                    values,
                }
            }
            "browser.press" => BrowserAction::Press {
                key: field(call, "key")?,
            },
            "browser.scroll" => BrowserAction::Scroll {
                x: field(call, "x")?,
                y: field(call, "y")?,
            },
            "browser.wait" => BrowserAction::Wait {
                condition: field(call, "condition")?,
                timeout_ms: field(call, "timeout_ms")?,
            },
            "browser.back" => BrowserAction::Back {},
            "browser.forward" => BrowserAction::Forward {},
            "browser.reload" => BrowserAction::Reload {},
            "browser.stop" => BrowserAction::Stop {},
            "browser.tab.open" => BrowserAction::TabOpen {
                url: optional(call, "url")?,
            },
            "browser.tab.select" => BrowserAction::TabSelect {
                tab_id: field(call, "tab_id")?,
            },
            "browser.tab.close" => BrowserAction::TabClose {
                tab_id: field(call, "tab_id")?,
            },
            _ => return Err(ToolError::Unknown(call.name.clone())),
        };
        Ok(Self::Action {
            session_id,
            control_generation,
            target,
            action,
        })
    }

    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Open { .. } => "browser.open",
            Self::Status {
                tabs_only: false, ..
            } => "browser.status",
            Self::Status {
                tabs_only: true, ..
            } => "browser.tabs",
            Self::Close { .. } => "browser.close",
            Self::Action { action, .. } => match action {
                BrowserAction::Navigate { .. } => "browser.navigate",
                BrowserAction::Snapshot { .. } => "browser.snapshot",
                BrowserAction::Click { .. } => "browser.click",
                BrowserAction::Fill { .. } => "browser.fill",
                BrowserAction::Select { .. } => "browser.select",
                BrowserAction::Press { .. } => "browser.press",
                BrowserAction::Scroll { .. } => "browser.scroll",
                BrowserAction::Wait { .. } => "browser.wait",
                BrowserAction::Back {} => "browser.back",
                BrowserAction::Forward {} => "browser.forward",
                BrowserAction::Reload {} => "browser.reload",
                BrowserAction::Stop {} => "browser.stop",
                BrowserAction::TabOpen { .. } => "browser.tab.open",
                BrowserAction::TabSelect { .. } => "browser.tab.select",
                BrowserAction::TabClose { .. } => "browser.tab.close",
            },
        }
    }

    pub(super) fn resource(&self) -> &str {
        match self {
            Self::Open { .. } => "browser-sessions",
            Self::Status { session_id, .. }
            | Self::Close { session_id, .. }
            | Self::Action { session_id, .. } => session_id.as_str(),
        }
    }
}

fn field<T: DeserializeOwned>(call: &ToolCall, name: &str) -> Result<T, ToolError> {
    decode(call, call.arguments.get(name).ok_or_else(|| invalid(call))?)
}

fn optional<T: DeserializeOwned>(call: &ToolCall, name: &str) -> Result<Option<T>, ToolError> {
    call.arguments
        .get(name)
        .map(|value| decode(call, value))
        .transpose()
}

fn decode<T: DeserializeOwned>(call: &ToolCall, value: &Value) -> Result<T, ToolError> {
    serde_json::from_value(value.clone()).map_err(|_| invalid(call))
}

fn invalid(call: &ToolCall) -> ToolError {
    ToolError::InvalidArguments {
        tool: call.name.clone(),
        message: "browser arguments violate the typed browser contract".into(),
    }
}
