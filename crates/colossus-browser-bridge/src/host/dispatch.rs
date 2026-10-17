//! Closed data/control vocabulary; unknown or wrong-channel frames fail closed.
use super::*;
impl Host {
    pub(super) fn fence_unknown(&self, id: &BrowserSessionId) {
        if let Ok(session) = self.existing(id)
            && let Ok(mut state) = session.state.lock()
        {
            state.authority.cancel();
            state.quiescing = true;
            state.closing |= self.enrollment.cancellation_closes_context;
            state.transfer = None;
            state.upload = None;
            state.download = None;
        }
    }

    pub(super) async fn dispatch(&self, request: Request) -> Response {
        let result = match request {
            Request::Ready {} => Ok(Response::Ready {}),
            Request::Open { request } => self
                .open(*request)
                .await
                .map(|tab| Response::Opened { tab }),
            Request::Execute { command } => self
                .execute(*command)
                .await
                .map(|observation| Response::Observed { observation }),
            Request::Capture { command } => self
                .capture(*command)
                .await
                .map(|descriptor| Response::Captured { descriptor }),
            Request::ReadScreenshot { request } => self
                .read_screenshot(*request)
                .await
                .map(|chunk| Response::ScreenshotChunk { chunk }),
            Request::BeginUpload { request } => self
                .prepare_upload(*request)
                .await
                .map(|receipt| Response::UploadPrepared { receipt }),
            Request::WriteUpload { request } => self
                .write_upload(*request)
                .await
                .map(|receipt| Response::UploadProgress { receipt }),
            Request::CommitUpload { request } => self
                .commit_upload(*request)
                .await
                .map(|observation| Response::Uploaded { observation }),
            Request::Download { command } => self
                .download(*command)
                .await
                .map(|descriptor| Response::Downloaded { descriptor }),
            Request::ReadDownload { request } => self
                .read_download(*request)
                .await
                .map(|chunk| Response::DownloadChunk { chunk }),
            Request::ConfirmNativeHandoff { request } => self
                .confirm_handoff(*request)
                .await
                .map(|tab| Response::NativeHandoffConfirmed { tab }),
            Request::Cancel {
                session_id,
                through_generation,
            } => self
                .cancel(&session_id, through_generation)
                .await
                .map(|()| Response::Acknowledged {}),
            Request::Close { session_id } => self
                .close(&session_id)
                .await
                .map(|()| Response::Acknowledged {}),
        };
        result.unwrap_or_else(Response::rejected)
    }
}
