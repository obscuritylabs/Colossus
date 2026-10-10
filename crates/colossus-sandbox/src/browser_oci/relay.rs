use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use super::process_identity::ProcessIdentity;
use colossus_ports::BrowserDriverError;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    net::{TcpStream, UnixListener, UnixStream},
    sync::oneshot,
    task::{JoinHandle, JoinSet},
};

pub(super) fn verify_peer(
    stream: &UnixStream,
    pid: u32,
    uid: u32,
    gid: u32,
) -> Result<(), BrowserDriverError> {
    let peer = stream.peer_cred().map_err(|_| BrowserDriverError::Denied)?;
    if peer.pid().and_then(|value| u32::try_from(value).ok()) != Some(pid)
        || peer.uid() != uid
        || peer.gid() != gid
        || pid == 0
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

pub(super) struct Relay {
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), BrowserDriverError>>>,
    terminal: Option<Result<(), BrowserDriverError>>,
}
impl Relay {
    pub(super) fn start(
        listener: UnixListener,
        address: SocketAddr,
        process: Arc<ProcessIdentity>,
        lifetime: Duration,
    ) -> Self {
        let (shutdown, receiver) = oneshot::channel();
        let task = tokio::spawn(serve(listener, address, process, lifetime, receiver));
        Self {
            shutdown: Some(shutdown),
            task: Some(task),
            terminal: None,
        }
    }
    pub(super) async fn revoke(&mut self) -> Result<(), BrowserDriverError> {
        if let Some(result) = self.terminal {
            return result;
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let result = match self.task.as_mut() {
            Some(task) => task
                .await
                .unwrap_or(Err(BrowserDriverError::OutcomeUnknown)),
            None => Err(BrowserDriverError::OutcomeUnknown),
        };
        self.task.take();
        self.terminal = Some(result);
        result
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn serve(
    listener: UnixListener,
    address: SocketAddr,
    process: Arc<ProcessIdentity>,
    lifetime: Duration,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), BrowserDriverError> {
    let mut connections = JoinSet::new();
    let deadline = tokio::time::Instant::now() + lifetime;
    let result = loop {
        tokio::select! {
            _ = &mut shutdown => break Ok(()),
            _ = tokio::time::sleep_until(deadline) => break Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(value) => value,
                    Err(_) => break Err(BrowserDriverError::OutcomeUnknown),
                };
                if process.verify_peer(&stream).is_err() || connections.len() >= 64 { drop(stream); continue; }
                connections.spawn(async move {
                    let target = TcpStream::connect(address).await?;
                    let (mut unix_reader, mut unix_writer) = stream.into_split();
                    let (mut tcp_reader, mut tcp_writer) = target.into_split();
                    let budget = AtomicU64::new(0);
                    let operation = async {
                        tokio::try_join!(copy(&mut unix_reader, &mut tcp_writer, &budget), copy(&mut tcp_reader, &mut unix_writer, &budget))?;
                        Ok::<(), io::Error>(())
                    };
                    tokio::time::timeout(Duration::from_secs(60), operation).await.map_err(|_| io::ErrorKind::TimedOut)?
                });
            }
            result = connections.join_next(), if !connections.is_empty() => { let _ = result; }
        }
    };
    drop(listener);
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    result
}

async fn copy(
    reader: &mut (impl AsyncRead + Unpin),
    writer: &mut (impl AsyncWrite + Unpin),
    budget: &AtomicU64,
) -> io::Result<()> {
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            writer.shutdown().await?;
            return Ok(());
        }
        if budget.fetch_add(count as u64, Ordering::AcqRel) + count as u64 > 32 * 1024 * 1024 {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        writer.write_all(&buffer[..count]).await?;
    }
}
