//! Anonymous inherited pipe halves with bounded cancellable native I/O workers.
mod thread;
use std::sync::Arc;
use std::time::Duration;
use std::{
    fs::File,
    future::Future as _,
    io::{self, Read as _, Write as _},
    pin::Pin,
    sync::{atomic::Ordering, mpsc as sync},
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{mpsc, oneshot},
};
use zeroize::Zeroizing;

const CHUNK: usize = 64 * 1024;
type WriteRequest = (Zeroizing<Vec<u8>>, oneshot::Sender<io::Result<usize>>);
/// Bounded anonymous-pipe reader with a retained cancellable worker.
pub struct PrivateReader {
    receiver: Option<mpsc::Receiver<io::Result<Zeroizing<Vec<u8>>>>>,
    pending: Zeroizing<Vec<u8>>,
    position: usize,
    thread: Arc<thread::Thread>,
}
/// Bounded anonymous-pipe writer acknowledging actual native writes.
pub struct PrivateWriter {
    sender: Option<sync::SyncSender<WriteRequest>>,
    pending: Option<oneshot::Receiver<io::Result<usize>>>,
    thread: Arc<thread::Thread>,
}
/// Async halves and a separate positive worker-drain obligation.
pub struct PrivatePipe {
    /// Bounded exclusively owned incoming half.
    pub reader: PrivateReader,
    /// Bounded exclusively owned outgoing half.
    pub writer: PrivateWriter,
    /// Keep until exact native reader/writer exit has been acknowledged.
    pub lease: PrivateIoLease,
}
/// Failed worker construction retaining any partially allocated join obligation.
pub struct PrivatePipeError {
    /// Native thread allocation failure.
    pub source: io::Error,
    /// The reader worker, if it existed before writer allocation failed.
    pub lease: Option<PrivateIoLease>,
}

impl PrivateReader {
    /// Transfer a private output pipe and retain its independent join obligation.
    ///
    /// # Errors
    /// Returns the native worker allocation error.
    pub fn from_file(file: File) -> io::Result<(Self, PrivateIoLease)> {
        let reader = Self::new(file)?;
        let lease = PrivateIoLease(vec![Arc::clone(&reader.thread)]);
        Ok((reader, lease))
    }
    fn new(mut file: File) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel(2);
        let thread = thread::Thread::start("colossus-browser-private-reader", move |cancelled| {
            while !cancelled.load(Ordering::Acquire) {
                let mut bytes = Zeroizing::new(vec![0; CHUNK]);
                let result = file.read(&mut bytes).map(|size| {
                    bytes.truncate(size);
                    bytes
                });
                let done = result.is_err() || result.as_ref().is_ok_and(|bytes| bytes.is_empty());
                if sender.blocking_send(result).is_err() || done {
                    break;
                }
            }
        })?;
        Ok(Self {
            receiver: Some(receiver),
            pending: Zeroizing::new(Vec::new()),
            position: 0,
            thread: Arc::new(thread),
        })
    }
}
impl AsyncRead for PrivateReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buffer.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.position == self.pending.len() {
            match self
                .receiver
                .as_mut()
                .expect("owned reader until drop")
                .poll_recv(context)
            {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(bytes))) => {
                    self.pending = bytes;
                    self.position = 0;
                }
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Err(error)),
                Poll::Ready(None) => return Poll::Ready(Ok(())),
            }
        }
        let size = buffer.remaining().min(self.pending.len() - self.position);
        buffer.put_slice(&self.pending[self.position..self.position + size]);
        let start = self.position;
        self.pending[start..start + size].fill(0);
        self.position += size;
        Poll::Ready(Ok(()))
    }
}
impl Drop for PrivateReader {
    fn drop(&mut self) {
        self.thread.cancelled.store(true, Ordering::Release);
        self.receiver.take(); // Wake a producer blocked on the bounded chunk queue.
        self.thread.cancel();
    }
}
impl PrivateWriter {
    fn new(mut file: File) -> io::Result<Self> {
        let (sender, receiver) = sync::sync_channel::<WriteRequest>(1);
        let thread = thread::Thread::start("colossus-browser-private-writer", move |cancelled| {
            while let Ok((bytes, acknowledge)) = receiver.recv() {
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                let result = file.write_all(&bytes).map(|()| bytes.len());
                let failed = result.is_err();
                let _ = acknowledge.send(result);
                if failed {
                    break;
                }
            }
        })?;
        Ok(Self {
            sender: Some(sender),
            pending: None,
            thread: Arc::new(thread),
        })
    }
    fn completed(&mut self, context: &mut Context<'_>) -> Poll<io::Result<usize>> {
        let Some(pending) = self.pending.as_mut() else {
            return Poll::Ready(Ok(0));
        };
        match Pin::new(pending).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.pending = None;
                Poll::Ready(result.unwrap_or_else(|_| Err(io::ErrorKind::BrokenPipe.into())))
            }
        }
    }
}
impl AsyncWrite for PrivateWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.pending.is_some() {
            return self.completed(context);
        }
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let bytes = Zeroizing::new(bytes[..bytes.len().min(CHUNK)].to_vec());
        let (send, receive) = oneshot::channel();
        if self
            .sender
            .as_ref()
            .is_none_or(|sender| sender.try_send((bytes, send)).is_err())
        {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        self.pending = Some(receive);
        self.completed(context)
    }
    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.completed(context).map_ok(|_| ())
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.completed(context) {
            Poll::Ready(result) => {
                self.sender.take();
                Poll::Ready(result.map(|_| ()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
impl Drop for PrivateWriter {
    fn drop(&mut self) {
        self.thread.cancelled.store(true, Ordering::Release);
        self.sender.take();
        self.pending.take();
        self.thread.cancel();
    }
}

/// Retained native worker identity. Cancelling a Rust future is not a drain receipt.
#[derive(Clone)]
pub struct PrivateIoLease(Vec<Arc<thread::Thread>>);
impl PrivateIoLease {
    /// Signal exact native workers without giving up the ownership obligation.
    pub fn cancel(&self) {
        for worker in &self.0 {
            worker.cancel();
        }
    }
    /// Cancel and positively join every exact retained native worker.
    /// A false result keeps the obligation available for retry.
    pub fn drain(&self, timeout: Duration) -> bool {
        self.cancel();
        let deadline = std::time::Instant::now() + timeout;
        let mut drained = true;
        for worker in &self.0 {
            drained &= worker.drain(deadline.saturating_duration_since(std::time::Instant::now()));
        }
        drained
    }
}
impl PrivatePipe {
    /// Move exclusively owned anonymous pipe halves into bounded native workers.
    ///
    /// # Errors
    /// Returns the native thread allocation error; no public endpoint is created.
    pub fn from_files(reader: File, writer: File) -> Result<Self, PrivatePipeError> {
        let reader = PrivateReader::new(reader).map_err(|source| PrivatePipeError {
            source,
            lease: None,
        })?;
        let writer = match PrivateWriter::new(writer) {
            Ok(writer) => writer,
            Err(source) => {
                let lease = PrivateIoLease(vec![Arc::clone(&reader.thread)]);
                drop(reader);
                return Err(PrivatePipeError {
                    source,
                    lease: Some(lease),
                });
            }
        };
        let lease = PrivateIoLease(vec![Arc::clone(&reader.thread), Arc::clone(&writer.thread)]);
        Ok(Self {
            reader,
            writer,
            lease,
        })
    }
}
