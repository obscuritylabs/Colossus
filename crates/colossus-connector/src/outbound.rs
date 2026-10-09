//! Keep native health and durable command receipts ahead of queued released output.
use colossus_cloud_protocol::{
    MAX_QUEUED_FRAMES,
    v1alpha1::{RuntimeFrame, runtime_frame},
};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc;
use tokio_stream::Stream;

const PRIORITY_FRAMES: usize = 4;

#[derive(Clone)]
pub(crate) struct FrameSender {
    priority: mpsc::Sender<RuntimeFrame>,
    data: mpsc::Sender<RuntimeFrame>,
}
pub(crate) struct FrameStream {
    priority: mpsc::Receiver<RuntimeFrame>,
    data: mpsc::Receiver<RuntimeFrame>,
}

pub(crate) fn channel() -> (FrameSender, FrameStream) {
    let (priority, priority_receiver) = mpsc::channel(PRIORITY_FRAMES);
    let (data, data_receiver) = mpsc::channel(MAX_QUEUED_FRAMES - PRIORITY_FRAMES);
    (
        FrameSender { priority, data },
        FrameStream {
            priority: priority_receiver,
            data: data_receiver,
        },
    )
}

impl FrameSender {
    pub(crate) async fn send(
        &self,
        frame: RuntimeFrame,
    ) -> Result<(), mpsc::error::SendError<RuntimeFrame>> {
        // Output-limit markers remain behind the data whose committed cursor they
        // reference. Only health and independent durable receipts may overtake it.
        let priority = matches!(
            &frame.body,
            Some(
                runtime_frame::Body::Hello(_)
                    | runtime_frame::Body::Heartbeat(_)
                    | runtime_frame::Body::Receipt(_)
            )
        );
        if priority {
            self.priority.send(frame).await
        } else {
            self.data.send(frame).await
        }
    }
}

impl Stream for FrameStream {
    type Item = RuntimeFrame;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let priority = self.priority.poll_recv(context);
        if let Poll::Ready(Some(frame)) = priority {
            return Poll::Ready(Some(frame));
        }
        match self.data.poll_recv(context) {
            Poll::Ready(Some(frame)) => Poll::Ready(Some(frame)),
            Poll::Ready(None) if priority.is_ready() => Poll::Ready(None),
            _ => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt;

    #[tokio::test]
    async fn saturated_data_queue_cannot_delay_native_health_or_receipts() {
        let (sender, mut stream) = channel();
        for _ in 0..MAX_QUEUED_FRAMES - PRIORITY_FRAMES {
            sender
                .send(RuntimeFrame {
                    body: Some(runtime_frame::Body::Snapshot(Default::default())),
                })
                .await
                .expect("fill bounded data");
        }
        sender
            .send(RuntimeFrame {
                body: Some(runtime_frame::Body::Heartbeat(Default::default())),
            })
            .await
            .expect("health slot");
        sender
            .send(RuntimeFrame {
                body: Some(runtime_frame::Body::Receipt(Default::default())),
            })
            .await
            .expect("receipt slot");
        assert!(matches!(
            stream.next().await.expect("health").body,
            Some(runtime_frame::Body::Heartbeat(_))
        ));
        assert!(matches!(
            stream.next().await.expect("receipt").body,
            Some(runtime_frame::Body::Receipt(_))
        ));
        assert!(matches!(
            stream.next().await.expect("data").body,
            Some(runtime_frame::Body::Snapshot(_))
        ));
        sender
            .send(RuntimeFrame {
                body: Some(runtime_frame::Body::OutputLimit(Default::default())),
            })
            .await
            .expect("ordered limit");
        for _ in 0..MAX_QUEUED_FRAMES - PRIORITY_FRAMES - 1 {
            assert!(matches!(
                stream.next().await.expect("data before limit").body,
                Some(runtime_frame::Body::Snapshot(_))
            ));
        }
        assert!(matches!(
            stream.next().await.expect("limit after data").body,
            Some(runtime_frame::Body::OutputLimit(_))
        ));
    }
}
