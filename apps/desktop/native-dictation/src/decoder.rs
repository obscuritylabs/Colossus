use std::{
    io::{BufRead as _, BufReader, Read as _, Write as _},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use zeroize::Zeroizing;

use crate::{DictationError, contract::SEGMENT_SAMPLES, pipeline::Decoder, worker::Reply};

pub(crate) struct WhisperDecoder {
    supervisor: Supervisor,
    input: ChildStdin,
    replies: Receiver<Result<Reply, DictationError>>,
    reader: Option<JoinHandle<()>>,
}

enum WatchCommand {
    Arm(Duration),
    Idle,
}

struct Supervisor {
    commands: Option<SyncSender<WatchCommand>>,
    watch: Option<JoinHandle<()>>,
}

impl Supervisor {
    fn new(mut child: Child, timeout: Duration) -> Self {
        let (commands, receiver) = mpsc::sync_channel(2);
        let watch = thread::spawn(move || {
            let mut deadline = Some(Instant::now() + timeout);
            loop {
                let command = if let Some(deadline) = deadline {
                    receiver
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .ok()
                } else {
                    receiver.recv().ok()
                };
                match command {
                    Some(WatchCommand::Arm(timeout)) => deadline = Some(Instant::now() + timeout),
                    Some(WatchCommand::Idle) => deadline = None,
                    None => break,
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        });
        Self {
            commands: Some(commands),
            watch: Some(watch),
        }
    }

    fn command(&self, command: WatchCommand) -> Result<(), DictationError> {
        self.commands
            .as_ref()
            .ok_or(DictationError::Inference)?
            .send(command)
            .map_err(|_| DictationError::Inference)
    }

    fn stop(&mut self) {
        self.commands.take();
        if let Some(watch) = self.watch.take() {
            let _ = watch.join();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

impl WhisperDecoder {
    pub(crate) fn load(path: &Path, digest: &str) -> Result<Self, DictationError> {
        let mut child =
            Command::new(std::env::current_exe().map_err(|_| DictationError::ModelUnsupported)?)
                .arg("--dictation-worker")
                .arg(path)
                .arg(digest)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| DictationError::ModelUnsupported)?;
        let Some(input) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DictationError::ModelUnsupported);
        };
        let Some(output) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DictationError::ModelUnsupported);
        };
        let (sender, replies) = mpsc::sync_channel(2);
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = String::new();
                let length = std::io::Read::by_ref(&mut output)
                    .take(64 * 1024)
                    .read_line(&mut line);
                let reply = match length {
                    Ok(1..=65535) if line.ends_with('\n') => {
                        serde_json::from_str(&line).map_err(|_| DictationError::Inference)
                    }
                    _ => Err(DictationError::Inference),
                };
                let failed = reply.is_err();
                if sender.send(reply).is_err() || failed {
                    break;
                }
            }
        });
        let decoder = Self {
            supervisor: Supervisor::new(child, Duration::from_secs(30)),
            input,
            replies,
            reader: Some(reader),
        };
        match decoder.replies.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(Reply::Ready)) => {
                decoder.supervisor.command(WatchCommand::Idle)?;
                Ok(decoder)
            }
            Ok(Ok(Reply::Failure { error })) => Err(error),
            _ => Err(DictationError::ModelUnsupported),
        }
    }
}

impl Decoder for WhisperDecoder {
    fn decode(&mut self, samples: &[f32]) -> Result<String, DictationError> {
        let count = u32::try_from(samples.len()).map_err(|_| DictationError::Inference)?;
        if samples.len() > SEGMENT_SAMPLES {
            return Err(DictationError::Inference);
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(4 + samples.len() * 4));
        bytes.extend_from_slice(&count.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        // Arm before writing: a stalled helper must not hold the controller
        // indefinitely even when the bounded audio frame exceeds pipe capacity.
        self.supervisor
            .command(WatchCommand::Arm(Duration::from_secs(10)))?;
        self.input
            .write_all(&bytes)
            .and_then(|()| self.input.flush())
            .map_err(|_| DictationError::Inference)?;
        let result = match self.replies.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(Reply::Text { text })) => Ok(text),
            Ok(Ok(Reply::Failure { error })) => Err(error),
            _ => Err(DictationError::Inference),
        };
        if result.is_ok() {
            self.supervisor.command(WatchCommand::Idle)?;
        }
        result
    }
}

impl Drop for WhisperDecoder {
    fn drop(&mut self) {
        self.supervisor.stop();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_terminates_and_reaps_a_stalled_native_helper() {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "decoder::tests::supervised_test_child",
                "--ignored",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let mut supervisor = Supervisor::new(child, Duration::from_millis(50));
        supervisor.watch.take().unwrap().join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(supervisor.command(WatchCommand::Idle).is_err());
    }

    #[test]
    #[ignore = "internal stalled child, launched only by the supervisor test"]
    fn supervised_test_child() {
        thread::sleep(Duration::from_mins(1));
    }
}
