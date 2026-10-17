//! Fixed private OCI entry; no renderer/model endpoint or general network relay.
use std::{
    io,
    net::TcpListener,
    os::unix::{fs::MetadataExt as _, net::UnixStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use colossus_ports::BrowserDriverError;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    task::JoinSet,
};

const ROOT: &str = "/run/colossus-browser-control";
const EGRESS: &str = "/run/colossus-browser-control/egress.sock";
const MAX_CONNECTIONS: usize = 64;
const MAX_BYTES: u64 = 32 * 1024 * 1024;

pub fn channels(presentation: bool) -> Result<Vec<UnixStream>, BrowserDriverError> {
    let root = Path::new(ROOT);
    let metadata = root
        .symlink_metadata()
        .map_err(|_| BrowserDriverError::Denied)?;
    // SAFETY: geteuid reads the current process effective UID without pointers.
    let owner = unsafe { libc::geteuid() };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || root
            .canonicalize()
            .map_err(|_| BrowserDriverError::Denied)?
            != root
    {
        return Err(BrowserDriverError::Denied);
    }
    // The supervisor proves network-namespace identity, immutable image and exact
    // process peer credentials before delivering the private bootstrap key.
    let entries =
        std::fs::read_dir("/sys/class/net").map_err(|_| BrowserDriverError::Unavailable)?;
    for entry in entries {
        if entry
            .map_err(|_| BrowserDriverError::Unavailable)?
            .file_name()
            != "lo"
        {
            return Err(BrowserDriverError::Denied);
        }
    }
    let mut names = vec!["bootstrap.sock", "data.sock", "control.sock"];
    if presentation {
        names.push("presentation.sock");
    }
    names
        .into_iter()
        .map(|name| UnixStream::connect(root.join(name)).map_err(|_| BrowserDriverError::Denied))
        .collect()
}

pub struct Relay {
    stopped: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<(), BrowserDriverError>>>,
}
impl Relay {
    pub fn start(cancelled: Arc<AtomicBool>) -> Result<Self, BrowserDriverError> {
        let listener =
            TcpListener::bind("127.0.0.1:18081").map_err(|_| BrowserDriverError::Unavailable)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| BrowserDriverError::Unavailable)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&cancelled);
        let thread_stopped = Arc::clone(&stopped);
        let thread = std::thread::spawn(move || {
            struct Stopped(Arc<AtomicBool>);
            impl Drop for Stopped {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let _stopped = Stopped(thread_stopped);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| BrowserDriverError::Unavailable)?;
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).map_err(|_| BrowserDriverError::Unavailable)?;
                let mut connections = JoinSet::new();
                let mut poll = tokio::time::interval(Duration::from_millis(5));
                loop {
                    tokio::select! {
                        _ = poll.tick() => { if thread_cancelled.load(Ordering::Acquire) { break; } }
                        accepted = listener.accept() => {
                            let (stream, _) = accepted.map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                            if connections.len() >= MAX_CONNECTIONS { drop(stream); continue; }
                            connections.spawn(async move {
                                let unix = tokio::net::UnixStream::connect(EGRESS).await?;
                                let (mut tcp_reader, mut tcp_writer) = stream.into_split();
                                let (mut unix_reader, mut unix_writer) = unix.into_split();
                                let budget = AtomicU64::new(0);
                                let future = async {
                                    tokio::try_join!(copy(&mut tcp_reader, &mut unix_writer, &budget), copy(&mut unix_reader, &mut tcp_writer, &budget))?;
                                    Ok::<(), io::Error>(())
                                };
                                tokio::time::timeout(Duration::from_secs(60), future).await.map_err(|_| io::ErrorKind::TimedOut)?
                            });
                        }
                        result = connections.join_next(), if !connections.is_empty() => { let _ = result; }
                    }
                }
                drop(listener);
                connections.abort_all();
                while connections.join_next().await.is_some() {}
                Ok(())
            })
        });
        Ok(Self {
            stopped,
            cancelled,
            thread: Some(thread),
        })
    }
    pub fn stop(&mut self) -> Result<(), BrowserDriverError> {
        self.cancelled.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.stopped.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)??;
        }
        Ok(())
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
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
        if budget.fetch_add(count as u64, Ordering::AcqRel) + count as u64 > MAX_BYTES {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        writer.write_all(&buffer[..count]).await?;
    }
}
