//! Bounded, owner-private operational logs shared by native application hosts.

use colossus_home::{ConfinedFile, ConfinedRoot};
use std::{
    io::{self, Seek as _, Write as _},
    path::Path,
};
use tracing_appender::non_blocking::{NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::{Layer as _, layer::SubscriberExt as _, util::SubscriberInitExt as _};

const LOG_FILES: usize = 4;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 32 * 1024;

/// Fixed operational verbosity. Dependency/HTTP wire logging cannot be enabled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LocalLogLevel {
    /// Successful lifecycle events and categorical failures.
    #[default]
    Info,
    /// Include bounded provider and native-operation progress metadata.
    Debug,
}

impl LocalLogLevel {
    /// Select `debug` with `COLOSSUS_LOG_LEVEL=debug`; otherwise use `info`.
    pub fn from_environment() -> Self {
        Self::from_value(std::env::var("COLOSSUS_LOG_LEVEL").ok().as_deref())
    }

    fn from_value(value: Option<&str>) -> Self {
        if value.is_some_and(|value| value.eq_ignore_ascii_case("debug")) {
            Self::Debug
        } else {
            Self::Info
        }
    }

    pub(crate) fn accepts(self, metadata: &tracing::Metadata<'_>) -> bool {
        let maximum = match self {
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
        };
        // Journal payload logging has its own explicit disclosure acknowledgement.
        // Never copy it, third-party diagnostics, or arbitrary parent-span fields
        // into the always-on local operational files.
        metadata.is_event()
            && *metadata.level() <= maximum
            && matches!(
                metadata.target(),
                "colossus.desktop"
                    | "colossus.provider"
                    | "colossus.computer"
                    | "colossus.browser"
                    | "colossus.lifecycle"
            )
    }
}

/// Host-owned nonblocking JSON log sink. Retain it until application work drains.
///
/// Four fixed 4 MiB files live under `logs/` in a validated private host root.
/// A retained file lock prevents two processes from truncating each other's logs.
/// Full queues drop records rather than blocking an agent or changing its outcome.
pub struct LocalLogSink {
    pub(crate) writer: NonBlocking,
    pub(crate) level: LocalLogLevel,
    _guard: WorkerGuard,
}

impl LocalLogSink {
    /// Open the fixed `desktop` or `sidecar` sink beneath native-owned storage.
    pub fn open(root: &ConfinedRoot, component: &str, level: LocalLogLevel) -> io::Result<Self> {
        let writer = RotatingWriter::open(root, component, MAX_FILE_BYTES)?;
        let (writer, guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(1024)
            .lossy(true)
            .thread_name("colossus-local-logs")
            .finish(writer);
        Ok(Self {
            writer,
            level,
            _guard: guard,
        })
    }

    /// Install a local-only subscriber in a native host without OTLP exporters.
    /// Hosts using exporters pass this sink to `ObservabilityGuard` instead.
    pub fn install(self) -> io::Result<Self> {
        let level = self.level;
        let layer = tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .with_span_list(false)
            .with_writer(self.writer.clone())
            .with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
                level.accepts(metadata)
            }));
        tracing_subscriber::registry()
            .with(layer)
            .try_init()
            .map_err(|_| io::Error::other("operational log subscriber is already installed"))?;
        Ok(self)
    }
}

struct RotatingWriter {
    root: ConfinedRoot,
    lock: ConfinedFile,
    file: ConfinedFile,
    component: String,
    slot: usize,
    bytes: u64,
    maximum: u64,
}

impl RotatingWriter {
    fn open(root: &ConfinedRoot, component: &str, maximum: u64) -> io::Result<Self> {
        if !matches!(component, "desktop" | "sidecar") {
            return Err(io::Error::other("unknown operational log component"));
        }
        let directory = root
            .prepare_directory(Path::new("logs"))
            .map_err(private_io)?;
        let root = ConfinedRoot::bind(directory).map_err(private_io)?;
        let lock = root
            .open_file(Path::new(&format!("{component}.lock")))
            .map_err(private_io)?;
        lock.file()
            .try_lock()
            .map_err(|_| io::Error::other("operational logs are already in use"))?;
        let file = root
            .open_file(Path::new(&format!("{component}-0.jsonl")))
            .map_err(private_io)?;
        let bytes = file.file().metadata()?.len();
        file.file().seek(io::SeekFrom::End(0))?;
        Ok(Self {
            root,
            lock,
            file,
            component: component.into(),
            slot: 0,
            bytes,
            maximum,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.file().flush()?;
        let slot = (self.slot + 1) % LOG_FILES;
        let file = self
            .root
            .open_file(Path::new(&format!("{}-{slot}.jsonl", self.component)))
            .map_err(private_io)?;
        file.revalidate(&self.root).map_err(private_io)?;
        file.file().set_len(0)?;
        file.file().seek(io::SeekFrom::Start(0))?;
        self.file = file;
        self.slot = slot;
        self.bytes = 0;
        Ok(())
    }
}

impl io::Write for RotatingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_RECORD_BYTES || bytes.len() as u64 > self.maximum {
            return Ok(bytes.len());
        }
        self.lock.revalidate(&self.root).map_err(private_io)?;
        self.file.revalidate(&self.root).map_err(private_io)?;
        if self.bytes.saturating_add(bytes.len() as u64) > self.maximum {
            self.rotate()?;
        }
        self.file.file().write_all(bytes)?;
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.file().flush()
    }
}

fn private_io(_: colossus_home::HomeError) -> io::Error {
    io::Error::other("operational log storage failed private-path validation")
}

#[cfg(test)]
mod tests;
