use super::*;

pub(super) struct ChannelWorkerObserver {
    pub(super) sender: tokio::sync::mpsc::Sender<WorkerFrameContent>,
}

#[async_trait]
impl RunEventObserver for ChannelWorkerObserver {
    async fn observe(&mut self, event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        self.sender
            .send(WorkerFrameContent::Event { event })
            .await
            .map_err(|_| ModelProviderError::Failed("worker event client disconnected".into()))
    }
}

pub(super) struct IpcRunObserver<'a, S> {
    pub(super) stream: &'a mut S,
    pub(super) key: &'a [u8; 32],
    pub(super) request_id: &'a str,
    pub(super) sequence: u64,
}

impl<S> IpcRunObserver<'_, S>
where
    S: AsyncWrite + Unpin + Send,
{
    pub(super) async fn complete(&mut self, result: Value) -> Result<(), WorkerError> {
        self.send(WorkerFrameContent::Complete { result }).await
    }

    pub(super) async fn error(&mut self, message: String) -> Result<(), WorkerError> {
        self.send(WorkerFrameContent::Error {
            message: bounded_error(&message),
        })
        .await
    }

    pub(super) async fn send(&mut self, content: WorkerFrameContent) -> Result<(), WorkerError> {
        self.sequence = self.sequence.saturating_add(1);
        write_signed_frame(
            self.stream,
            self.key,
            self.request_id,
            self.sequence,
            content,
        )
        .await
    }
}

#[async_trait]
impl<S> colossus_ports::RunEventObserver for IpcRunObserver<'_, S>
where
    S: AsyncWrite + Unpin + Send,
{
    async fn observe(
        &mut self,
        event: RunEventEnvelope,
    ) -> Result<(), colossus_ports::ModelProviderError> {
        self.send(WorkerFrameContent::Event { event })
            .await
            .map_err(|error| colossus_ports::ModelProviderError::Failed(error.to_string()))
    }
}
