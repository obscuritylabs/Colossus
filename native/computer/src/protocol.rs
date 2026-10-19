//! Closed private helper protocol. Neither role nor native identity is model input.
use crate::{TrustedComputerContext, wire};
use colossus_contracts::*;
use colossus_ports::*;
use serde::{Deserialize, Serialize};

pub(crate) const HELPER_ARGUMENT: &str = "__computer-helper-v1";
pub(crate) const VERSION: u32 = 1;

pub(crate) fn opaque(prefix: &str) -> Result<String, ComputerDriverError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| ComputerDriverError::Failed)?;
    Ok(format!(
        "{prefix}{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bootstrap {
    pub version: u32,
    pub nonce: String,
    pub context: TrustedComputerContext,
    pub binding: ComputerBinding,
    pub role: Role,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Role {
    Broker,
    #[cfg(windows)]
    Read {
        window: crate::platform::identity::Window,
        query: ReadQuery,
    },
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ReadQuery {
    Snapshot { max_nodes: u16 },
    Screenshot { max_bytes: u32, max_pixels: u32 },
    ProtectedFocus,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub nonce: String,
    pub sequence: u64,
    pub request: Option<ComputerDriverRequest>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Response {
    pub nonce: String,
    pub sequence: u64,
    pub result: Result<Reply, ComputerDriverError>,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "reply",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum Reply {
    Ready,
    Windows(Vec<ComputerWindowSummary>),
    Opened(ComputerSessionSummary),
    Status(ComputerSessionSummary),
    Snapshot(ComputerObservation),
    Screenshot {
        descriptor: ComputerScreenshotDescriptor,
        root_window_id: ComputerWindowId,
    },
    Applied(ComputerSessionSummary),
    Closed,
    Nodes(Vec<ComputerNode>),
    Pixels,
    ProtectedFocus(bool),
}

impl Reply {
    pub fn into_public(self, binary: Vec<u8>) -> Result<ComputerDriverReply, ComputerDriverError> {
        if !matches!(self, Self::Screenshot { .. }) && !binary.is_empty() {
            return Err(ComputerDriverError::Failed);
        }
        Ok(match self {
            Self::Windows(v) => ComputerDriverReply::Windows(v),
            Self::Opened(v) => ComputerDriverReply::Opened(v),
            Self::Status(v) => ComputerDriverReply::Status(v),
            Self::Snapshot(v) => ComputerDriverReply::Snapshot(v),
            Self::Screenshot {
                descriptor,
                root_window_id,
            } => {
                if binary.is_empty()
                    || binary.len() > wire::MAX_PNG
                    || binary.len() != descriptor.size_bytes as usize
                {
                    return Err(ComputerDriverError::Failed);
                }
                ComputerDriverReply::Screenshot(ComputerScreenshotCapture {
                    descriptor,
                    root_window_id,
                    png: binary,
                })
            }
            Self::Applied(v) => ComputerDriverReply::Applied(v),
            Self::Closed => ComputerDriverReply::Closed,
            _ => return Err(ComputerDriverError::Failed),
        })
    }
}
