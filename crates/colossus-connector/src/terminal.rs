//! Independent human terminal admission through the dedicated public SDK client.
use colossus_cloud_protocol::{
    MAX_TERMINAL_CONTROL_REQUESTS, MAX_TERMINAL_PAYLOAD_BYTES, MAX_TERMINAL_READ_REQUESTS,
    TerminalReply, decode, encode,
    v1alpha1::{TerminalRequest, TerminalResponse},
    valid_terminal_request_id,
};
use colossus_sdk::{AgentRunClient, HumanTerminalOperation, HumanTerminalRequest};
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::Status;

pub(crate) async fn available(client: &dyn AgentRunClient) -> bool {
    matches!(tokio::time::timeout(Duration::from_secs(2), client.human_terminal(HumanTerminalRequest { owner: "fleet.context".into(), operation: HumanTerminalOperation::Context })).await, Ok(Ok(reply)) if reply.available)
}

pub(crate) struct DecodedRequest {
    id: String,
    request: HumanTerminalRequest,
}
impl DecodedRequest {
    pub(crate) fn is_read(&self) -> bool {
        matches!(&self.request.operation, HumanTerminalOperation::Read { .. })
    }
}
pub(crate) struct Admission {
    reads: Arc<Semaphore>,
    controls: Arc<Semaphore>,
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            reads: Arc::new(Semaphore::new(MAX_TERMINAL_READ_REQUESTS)),
            controls: Arc::new(Semaphore::new(MAX_TERMINAL_CONTROL_REQUESTS)),
        }
    }
}
impl Admission {
    pub(crate) fn acquire(&self, read: bool) -> Result<OwnedSemaphorePermit, Status> {
        (if read { &self.reads } else { &self.controls })
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("terminal request lane bound"))
    }
}
pub(crate) fn decode_request(request: TerminalRequest) -> Result<DecodedRequest, Status> {
    if !valid_terminal_request_id(&request.request_id)
        || request.request_json.len() > MAX_TERMINAL_PAYLOAD_BYTES
    {
        return Err(Status::invalid_argument("invalid terminal envelope"));
    }
    let input: HumanTerminalRequest = decode(&request.request_json)
        .map_err(|_| Status::invalid_argument("invalid terminal operation"))?;
    if !input.validate() {
        return Err(Status::invalid_argument("invalid terminal operation"));
    }
    Ok(DecodedRequest {
        id: request.request_id,
        request: input,
    })
}
pub(crate) async fn execute(
    client: &dyn AgentRunClient,
    request: DecodedRequest,
) -> Result<TerminalResponse, Status> {
    let reply = match client.human_terminal(request.request).await {
        Ok(reply) => TerminalReply::Result { reply },
        Err(error) => TerminalReply::Failed { error },
    };
    let reply_json = encode(&reply)
        .map_err(|_| Status::resource_exhausted("terminal response exceeds its bound"))?;
    if reply_json.len() > MAX_TERMINAL_PAYLOAD_BYTES {
        return Err(Status::resource_exhausted(
            "terminal response exceeds its bound",
        ));
    }
    Ok(TerminalResponse {
        request_id: request.id,
        reply_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eight_waiting_reads_cannot_consume_interrupt_or_close_capacity() {
        let admission = Admission::default();
        let reads = (0..MAX_TERMINAL_READ_REQUESTS)
            .map(|_| admission.acquire(true).unwrap())
            .collect::<Vec<_>>();
        assert!(admission.acquire(true).is_err());
        let controls = (0..MAX_TERMINAL_CONTROL_REQUESTS)
            .map(|_| admission.acquire(false).unwrap())
            .collect::<Vec<_>>();
        assert!(admission.acquire(false).is_err());
        drop(controls);
        assert!(admission.acquire(false).is_ok());
        assert_eq!(reads.len(), MAX_TERMINAL_READ_REQUESTS);
    }
}
