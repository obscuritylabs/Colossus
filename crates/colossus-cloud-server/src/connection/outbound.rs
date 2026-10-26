//! Independent terminal queue; human input cannot consume run/control slots.
use colossus_cloud_protocol::{
    MAX_QUEUED_FRAMES, MAX_TERMINAL_QUEUED_FRAMES, v1alpha1::ControlFrame,
};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tonic::Status;

#[derive(Clone)]
pub(crate) struct FrameSender {
    control: mpsc::Sender<Result<ControlFrame, Status>>,
    terminal: mpsc::Sender<Result<ControlFrame, Status>>,
}
pub(crate) struct FrameStream {
    control: mpsc::Receiver<Result<ControlFrame, Status>>,
    terminal: mpsc::Receiver<Result<ControlFrame, Status>>,
    control_burst: u8,
}
pub(crate) fn channel() -> (FrameSender, FrameStream) {
    let (control, control_receiver) = mpsc::channel(MAX_QUEUED_FRAMES);
    let (terminal, terminal_receiver) = mpsc::channel(MAX_TERMINAL_QUEUED_FRAMES);
    (
        FrameSender { control, terminal },
        FrameStream {
            control: control_receiver,
            terminal: terminal_receiver,
            control_burst: 0,
        },
    )
}
impl FrameSender {
    pub(crate) async fn send(
        &self,
        frame: Result<ControlFrame, Status>,
    ) -> Result<(), mpsc::error::SendError<Result<ControlFrame, Status>>> {
        self.control.send(frame).await
    }
    pub(crate) fn try_terminal(&self, frame: ControlFrame) -> Result<(), ()> {
        self.terminal.try_send(Ok(frame)).map_err(|_| ())
    }
}
impl Stream for FrameStream {
    type Item = Result<ControlFrame, Status>;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.control_burst >= 8
            && let Poll::Ready(Some(frame)) = self.terminal.poll_recv(context)
        {
            self.control_burst = 0;
            return Poll::Ready(Some(frame));
        }
        let control = self.control.poll_recv(context);
        if let Poll::Ready(Some(frame)) = control {
            self.control_burst = self.control_burst.saturating_add(1);
            return Poll::Ready(Some(frame));
        }
        match self.terminal.poll_recv(context) {
            Poll::Ready(Some(frame)) => {
                self.control_burst = 0;
                Poll::Ready(Some(frame))
            }
            Poll::Ready(None) if control.is_ready() => Poll::Ready(None),
            _ => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_cloud_protocol::v1alpha1::{TerminalRequest, control_frame};
    use futures::StreamExt;
    #[tokio::test]
    async fn saturated_terminal_lane_cannot_delay_control_or_consume_its_slots() {
        let (sender, mut stream) = channel();
        for _ in 0..MAX_TERMINAL_QUEUED_FRAMES {
            sender
                .try_terminal(ControlFrame {
                    body: Some(control_frame::Body::TerminalRequest(
                        TerminalRequest::default(),
                    )),
                })
                .unwrap();
        }
        assert!(sender.try_terminal(ControlFrame::default()).is_err());
        sender
            .send(Ok(ControlFrame {
                body: Some(control_frame::Body::Acknowledgement(Default::default())),
            }))
            .await
            .unwrap();
        assert!(matches!(
            stream.next().await.unwrap().unwrap().body,
            Some(control_frame::Body::Acknowledgement(_))
        ));
        assert!(matches!(
            stream.next().await.unwrap().unwrap().body,
            Some(control_frame::Body::TerminalRequest(_))
        ));
    }
}
