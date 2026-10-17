use colossus_contracts::{BrowserObservation, BrowserSessionId, BrowserTabSummary};
use colossus_ports::{
    BrowserDownloadDescriptor, BrowserDownloadReadRequest, BrowserDriverCommand,
    BrowserDriverError, BrowserDriverOpenRequest, BrowserNativeHandoffRequest,
    BrowserScreenshotChunk, BrowserScreenshotDescriptor, BrowserScreenshotReadRequest,
    BrowserUploadCommitRequest, BrowserUploadPrepareRequest, BrowserUploadReceipt,
    BrowserUploadWriteRequest,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Ready {},
    Open {
        request: Box<BrowserDriverOpenRequest>,
    },
    Execute {
        command: Box<BrowserDriverCommand>,
    },
    Capture {
        command: Box<BrowserDriverCommand>,
    },
    ReadScreenshot {
        request: Box<BrowserScreenshotReadRequest>,
    },
    BeginUpload {
        request: Box<BrowserUploadPrepareRequest>,
    },
    WriteUpload {
        request: Box<BrowserUploadWriteRequest>,
    },
    CommitUpload {
        request: Box<BrowserUploadCommitRequest>,
    },
    Download {
        command: Box<BrowserDriverCommand>,
    },
    ReadDownload {
        request: Box<BrowserDownloadReadRequest>,
    },
    ConfirmNativeHandoff {
        request: Box<BrowserNativeHandoffRequest>,
    },
    Cancel {
        session_id: BrowserSessionId,
        through_generation: u64,
    },
    Close {
        session_id: BrowserSessionId,
    },
}

impl Request {
    pub(crate) fn session(&self) -> Option<&BrowserSessionId> {
        match self {
            Self::Ready {} => None,
            Self::Open { request } => Some(&request.session_id),
            Self::Execute { command } | Self::Capture { command } | Self::Download { command } => {
                Some(&command.session_id)
            }
            Self::ReadScreenshot { request }
            | Self::CommitUpload { request }
            | Self::ReadDownload { request } => Some(&request.session_id),
            Self::BeginUpload { request } => Some(&request.command.session_id),
            Self::WriteUpload { request } => Some(&request.transfer.session_id),
            Self::ConfirmNativeHandoff { request } => Some(&request.session_id),
            Self::Cancel { session_id, .. } | Self::Close { session_id } => Some(session_id),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Response {
    Ready {},
    Opened {
        tab: BrowserTabSummary,
    },
    Observed {
        observation: BrowserObservation,
    },
    Captured {
        descriptor: BrowserScreenshotDescriptor,
    },
    ScreenshotChunk {
        chunk: BrowserScreenshotChunk,
    },
    UploadPrepared {
        receipt: BrowserUploadReceipt,
    },
    UploadProgress {
        receipt: BrowserUploadReceipt,
    },
    Uploaded {
        observation: BrowserObservation,
    },
    Downloaded {
        descriptor: BrowserDownloadDescriptor,
    },
    DownloadChunk {
        chunk: BrowserScreenshotChunk,
    },
    NativeHandoffConfirmed {
        tab: BrowserTabSummary,
    },
    Acknowledged {},
    Rejected {
        code: ErrorCode,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorCode {
    Unavailable,
    Unsupported,
    Denied,
    Stale,
    Cancelled,
    LimitExceeded,
    AuthenticationRequired,
    Failed,
    OutcomeUnknown,
}

impl From<BrowserDriverError> for ErrorCode {
    fn from(value: BrowserDriverError) -> Self {
        match value {
            BrowserDriverError::Unavailable => Self::Unavailable,
            BrowserDriverError::Unsupported => Self::Unsupported,
            BrowserDriverError::Denied => Self::Denied,
            BrowserDriverError::Stale => Self::Stale,
            BrowserDriverError::Cancelled => Self::Cancelled,
            BrowserDriverError::LimitExceeded => Self::LimitExceeded,
            BrowserDriverError::AuthenticationRequired => Self::AuthenticationRequired,
            BrowserDriverError::Failed => Self::Failed,
            BrowserDriverError::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

impl From<ErrorCode> for BrowserDriverError {
    fn from(value: ErrorCode) -> Self {
        match value {
            ErrorCode::Unavailable => Self::Unavailable,
            ErrorCode::Unsupported => Self::Unsupported,
            ErrorCode::Denied => Self::Denied,
            ErrorCode::Stale => Self::Stale,
            ErrorCode::Cancelled => Self::Cancelled,
            ErrorCode::LimitExceeded => Self::LimitExceeded,
            ErrorCode::AuthenticationRequired => Self::AuthenticationRequired,
            ErrorCode::Failed => Self::Failed,
            ErrorCode::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

impl Response {
    pub(crate) fn rejected(error: BrowserDriverError) -> Self {
        Self::Rejected { code: error.into() }
    }
}
