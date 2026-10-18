//! Trusted owner registry and independently supervised helper lifetime.
use crate::{
    TrustedComputerContext,
    protocol::{self, Bootstrap, Reply, Request, Response, Role},
    wire,
};
use async_trait::async_trait;
use colossus_contracts::{ComputerBinding, ComputerCapabilities, ComputerLimits};
use colossus_ports::{
    ComputerDriver, ComputerDriverError as Error, ComputerDriverReply, ComputerDriverRequest,
    RunControl,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

/// Verified native host adapter. Construction alone grants no model capability.
pub struct NativeComputerDriver {
    executable: PathBuf,
    #[cfg(windows)]
    image: Arc<colossus_windows_native::BoundPath>,
    context: TrustedComputerContext,
    owners: crate::owner_registry::OwnerRegistry<Owner>,
}
struct Owner {
    revoked: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    #[cfg(feature = "native-test-driver")]
    job: Arc<Mutex<Option<Arc<colossus_windows_native::KillOnCloseJob>>>>,
    sender: mpsc::Sender<Message>,
}
struct Supervision {
    revoked: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    job: Arc<Mutex<Option<Arc<colossus_windows_native::KillOnCloseJob>>>>,
}
enum Message {
    Execute(
        Box<ComputerDriverRequest>,
        RunControl,
        tokio::sync::oneshot::Sender<Result<ComputerDriverReply, Error>>,
    ),
    Close(tokio::sync::oneshot::Sender<Result<(), Error>>),
}

impl NativeComputerDriver {
    /// Install only the exact already-verified current CLI/sidecar executable.
    /// Composition supplies protected host/controller process identities.
    pub fn new(executable: PathBuf, context: TrustedComputerContext) -> Result<Self, Error> {
        if !executable.is_absolute() || context.protected.is_empty() || context.protected.len() > 32
        {
            return Err(Error::InvalidInput);
        }
        #[cfg(windows)]
        let image = {
            let image = colossus_windows_native::BoundPath::open_immutable_file(&executable)
                .map_err(|_| Error::Denied)?;
            let current = std::env::current_exe().map_err(|_| Error::Denied)?;
            let current = colossus_windows_native::BoundPath::open_immutable_file(&current)
                .map_err(|_| Error::Denied)?;
            if image.identity() != current.identity() {
                return Err(Error::Denied);
            }
            Arc::new(image)
        };
        crate::platform::identity::ordinary_host().map_err(|_| Error::Unavailable)?;
        Ok(Self {
            executable,
            #[cfg(windows)]
            image,
            context,
            owners: crate::owner_registry::OwnerRegistry::new(),
        })
    }
    fn owner(&self, binding: &ComputerBinding, control: &RunControl) -> Result<Arc<Owner>, Error> {
        self.owners.admit(binding, control, || {
            let (sender, receiver) = mpsc::channel(8);
            let revoked = Arc::new(AtomicBool::new(false));
            let closed = Arc::new(AtomicBool::new(false));
            let job = Arc::new(Mutex::new(None));
            let owner = Arc::new(Owner {
                sender,
                revoked: Arc::clone(&revoked),
                closed: Arc::clone(&closed),
                #[cfg(feature = "native-test-driver")]
                job: Arc::clone(&job),
            });
            #[cfg(windows)]
            tokio::spawn(actor(
                self.executable.clone(),
                Arc::clone(&self.image),
                self.context.clone(),
                binding.clone(),
                Supervision {
                    revoked,
                    closed,
                    job,
                },
                receiver,
            ));
            #[cfg(not(windows))]
            {
                drop(receiver);
                let _ = revoked;
            }
            owner
        })
    }
    /// Abort this driver's own helper only during explicit disposable acceptance.
    /// This surface is absent from normal host builds and model tool APIs.
    #[cfg(feature = "native-test-driver")]
    pub fn acceptance_abort_helper(&self, binding: &ComputerBinding) -> Result<(), Error> {
        let owner = self.owners.existing(binding)?.ok_or(Error::Stale)?;
        let job = owner
            .job
            .lock()
            .map_err(|_| Error::Failed)?
            .clone()
            .ok_or(Error::Stale)?;
        job.terminate().map_err(|_| Error::Failed)
    }
}
#[async_trait]
impl ComputerDriver for NativeComputerDriver {
    fn capabilities(&self) -> ComputerCapabilities {
        let available = cfg!(windows);
        ComputerCapabilities {
            available,
            platform: std::env::consts::OS.into(),
            limits: ComputerLimits::HARD_MAX,
            unavailable_reason: (!available).then(|| "unsupported_platform".into()),
        }
    }
    async fn execute(
        &self,
        request: ComputerDriverRequest,
        control: &RunControl,
    ) -> Result<ComputerDriverReply, Error> {
        request.validate().map_err(|_| Error::InvalidInput)?;
        if control.is_cancelled() {
            self.revoke_run(&request.binding);
            return Err(Error::Denied);
        }
        let owner = self.owner(&request.binding, control)?;
        if owner.revoked.load(Ordering::Acquire) {
            return Err(Error::Denied);
        }
        let (sender, response) = tokio::sync::oneshot::channel();
        owner
            .sender
            .try_send(Message::Execute(Box::new(request), control.clone(), sender))
            .map_err(|_| Error::Busy)?;
        response.await.map_err(|_| Error::OutcomeUnknown)?
    }
    fn revoke_run(&self, binding: &ComputerBinding) {
        self.owners.revoke(binding, |owner| {
            owner.revoked.store(true, Ordering::Release)
        });
    }
    async fn close_run(&self, binding: &ComputerBinding) -> Result<(), Error> {
        self.revoke_run(binding);
        let Some(owner) = self.owners.existing(binding)? else {
            return Ok(());
        };
        if owner.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let (sender, response) = tokio::sync::oneshot::channel();
        if owner.sender.send(Message::Close(sender)).await.is_err() {
            return if owner.closed.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(Error::OutcomeUnknown)
            };
        }
        tokio::time::timeout(Duration::from_secs(3), response)
            .await
            .map_err(|_| Error::OutcomeUnknown)?
            .map_err(|_| Error::OutcomeUnknown)?
    }
}

#[cfg(windows)]
struct Child {
    child: tokio::process::Child,
    input: Option<tokio::process::ChildStdin>,
    output: Option<tokio::process::ChildStdout>,
    job: Option<Arc<colossus_windows_native::KillOnCloseJob>>,
    ready: bool,
    nonce: String,
    sequence: u64,
}
#[cfg(windows)]
impl Child {
    fn spawn(path: &PathBuf, image: &colossus_windows_native::BoundPath) -> Result<Self, Error> {
        image.revalidate().map_err(|_| Error::Unavailable)?;
        let nonce = protocol::opaque("cu_p_")?;
        let mut command = tokio::process::Command::new(path);
        command
            .arg(protocol::HELPER_ARGUMENT)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(if cfg!(feature = "native-test-driver") {
                std::process::Stdio::inherit()
            } else {
                std::process::Stdio::null()
            })
            .kill_on_drop(true);
        crate::platform::configure_environment(command.as_std_mut())?;
        colossus_windows_native::configure_suspended_process(command.as_std_mut());
        let mut child = command.spawn().map_err(|_| Error::Unavailable)?;
        let input = child.stdin.take();
        let output = child.stdout.take();
        Ok(Self {
            child,
            input,
            output,
            job: None,
            ready: false,
            nonce,
            sequence: 0,
        })
    }
    fn enroll(&mut self, image: &colossus_windows_native::BoundPath) -> Result<(), Error> {
        let (job, _) =
            colossus_windows_native::KillOnCloseJob::assign_tokio_child_verify_and_resume(
                &self.child,
                image.identity(),
            )
            .map_err(|_| Error::Unavailable)?;
        self.job = Some(Arc::new(job));
        Ok(())
    }
    async fn initialize(
        &mut self,
        context: TrustedComputerContext,
        binding: ComputerBinding,
        role: Role,
    ) -> Result<(), Error> {
        use tokio::io::AsyncWriteExt as _;
        self.input
            .as_mut()
            .ok_or(Error::Failed)?
            .write_all(
                &wire::encode(
                    &Bootstrap {
                        version: protocol::VERSION,
                        nonce: self.nonce.clone(),
                        context,
                        binding,
                        role,
                    },
                    &[],
                )
                .map_err(|_| Error::Failed)?,
            )
            .await
            .map_err(|_| Error::Failed)?;
        let (reply, binary) = self.read().await?;
        if reply.sequence != 0
            || reply.nonce != self.nonce
            || !binary.is_empty()
            || !matches!(reply.result, Ok(Reply::Ready))
        {
            return Err(Error::Failed);
        }
        self.ready = true;
        Ok(())
    }
    async fn read(&mut self) -> Result<(Response, Vec<u8>), Error> {
        use tokio::io::AsyncReadExt as _;
        let mut header = [0; 8];
        self.output
            .as_mut()
            .ok_or(Error::Failed)?
            .read_exact(&mut header)
            .await
            .map_err(|_| Error::OutcomeUnknown)?;
        let json = u32::from_le_bytes(header[..4].try_into().map_err(|_| Error::Failed)?) as usize;
        let binary =
            u32::from_le_bytes(header[4..].try_into().map_err(|_| Error::Failed)?) as usize;
        if json == 0 || json > wire::MAX_JSON || binary > wire::MAX_PNG {
            return Err(Error::Failed);
        }
        let mut frame = vec![0; json + binary];
        self.output
            .as_mut()
            .ok_or(Error::Failed)?
            .read_exact(&mut frame)
            .await
            .map_err(|_| Error::OutcomeUnknown)?;
        let response = serde_json::from_slice(&frame[..json]).map_err(|_| Error::Failed)?;
        Ok((response, frame.split_off(json)))
    }
    async fn request(
        &mut self,
        request: Option<ComputerDriverRequest>,
    ) -> Result<(Reply, Vec<u8>), Error> {
        use tokio::io::AsyncWriteExt as _;
        if matches!(
            request.as_ref().map(|r| &r.command),
            Some(colossus_ports::ComputerCommand::Input {
                input: colossus_contracts::ComputerInput::Focus { .. },
                ..
            })
        ) && let Some(pid) = self.child.id()
        {
            // SAFETY: only our retained, image-verified, job-bound helper PID.
            // This forwards existing Windows foreground eligibility; refusal
            // remains a normal, transparent result of the native Focus action.
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid) };
        }
        self.sequence = self.sequence.checked_add(1).ok_or(Error::Failed)?;
        let frame = wire::encode(
            &Request {
                nonce: self.nonce.clone(),
                sequence: self.sequence,
                request,
            },
            &[],
        )
        .map_err(|_| Error::InvalidInput)?;
        self.input
            .as_mut()
            .ok_or(Error::Failed)?
            .write_all(&frame)
            .await
            .map_err(|_| Error::OutcomeUnknown)?;
        let (response, binary) = self.read().await?;
        if response.nonce != self.nonce || response.sequence != self.sequence {
            return Err(Error::OutcomeUnknown);
        }
        Ok((response.result?, binary))
    }
    async fn shutdown(&mut self) -> Result<(), Error> {
        let result = if self.ready {
            tokio::time::timeout(Duration::from_secs(2), self.request(None))
                .await
                .ok()
        } else {
            None
        };
        if !matches!(result, Some(Ok((Reply::Closed, _)))) {
            if let Some(job) = &self.job {
                job.terminate().map_err(|_| Error::OutcomeUnknown)?;
            } else if self
                .child
                .try_wait()
                .map_err(|_| Error::OutcomeUnknown)?
                .is_none()
            {
                self.child.start_kill().map_err(|_| Error::OutcomeUnknown)?;
            }
            tokio::time::timeout(Duration::from_secs(1), self.child.wait())
                .await
                .map_err(|_| Error::OutcomeUnknown)?
                .map_err(|_| Error::OutcomeUnknown)?;
            // Balanced native input has no cross-call holds. Kernel-confirmed
            // helper exit proves cleanup even when its final ACK was lost.
            return Ok(());
        }
        tokio::time::timeout(Duration::from_secs(1), self.child.wait())
            .await
            .map_err(|_| Error::OutcomeUnknown)?
            .map_err(|_| Error::Failed)?;
        Ok(())
    }
}

#[cfg(windows)]
async fn actor(
    path: PathBuf,
    image: Arc<colossus_windows_native::BoundPath>,
    context: TrustedComputerContext,
    binding: ComputerBinding,
    supervision: Supervision,
    mut receiver: mpsc::Receiver<Message>,
) {
    let Supervision {
        revoked,
        closed,
        job: active_job,
    } = supervision;
    let mut child: Option<Child> = None;
    loop {
        let message = tokio::select! {
            message=receiver.recv()=>message,
            _=tokio::time::sleep(Duration::from_millis(25))=>{
                if revoked.load(Ordering::Acquire) && child.is_some() {
                    let _=cleanup_owned(&mut child,&active_job).await;
                }
                continue
            }
        };
        let Some(message) = message else { break };
        match message {
            Message::Close(reply) => {
                let result = cleanup_owned(&mut child, &active_job).await;
                closed.store(result.is_ok(), Ordering::Release);
                let _ = reply.send(result);
                if closed.load(Ordering::Acquire) {
                    break;
                }
            }
            Message::Execute(request, control, reply) => {
                if revoked.load(Ordering::Acquire) || control.is_cancelled() {
                    let _ = reply.send(Err(Error::Denied));
                    continue;
                }
                if child.is_none() {
                    match Child::spawn(&path, &image) {
                        Ok(process) => child = Some(process),
                        Err(error) => {
                            let _ = reply.send(Err(error));
                            continue;
                        }
                    }
                    // Custody exists before image/job enrollment or any async
                    // bootstrap. Failed-start cleanup must retain the same PID.
                    let enrollment = child
                        .as_mut()
                        .ok_or(Error::Failed)
                        .and_then(|process| process.enroll(&image));
                    if let Some(process) = &child
                        && let Ok(mut job) = active_job.lock()
                    {
                        *job = process.job.clone();
                    }
                    let initialization = if enrollment.is_ok() {
                        if let Some(process) = child.as_mut() {
                            tokio::time::timeout(
                                Duration::from_secs(5),
                                process.initialize(context.clone(), binding.clone(), Role::Broker),
                            )
                            .await
                            .map_err(|_| Error::Unavailable)
                            .and_then(|r| r)
                        } else {
                            Err(Error::Failed)
                        }
                    } else {
                        enrollment
                    };
                    if initialization.is_err() {
                        revoked.store(true, Ordering::Release);
                        let _ = cleanup_owned(&mut child, &active_job).await;
                        let _ = reply.send(Err(Error::Unavailable));
                        continue;
                    }
                }
                if revoked.load(Ordering::Acquire) || control.is_cancelled() || reply.is_closed() {
                    revoked.store(true, Ordering::Release);
                    let _ = cleanup_owned(&mut child, &active_job).await;
                    let _ = reply.send(Err(Error::Denied));
                    continue;
                }
                let Some(process) = child.as_mut() else {
                    let _ = reply.send(Err(Error::Unavailable));
                    continue;
                };
                let result = {
                    let operation = process.request(Some(*request));
                    tokio::pin!(operation);
                    let deadline = tokio::time::sleep(Duration::from_secs(10));
                    tokio::pin!(deadline);
                    loop {
                        tokio::select! {value=&mut operation=>break value,_=&mut deadline=>break Err(Error::OutcomeUnknown),_=tokio::time::sleep(Duration::from_millis(10))=>{
                            if revoked.load(Ordering::Acquire)||control.is_cancelled()||reply.is_closed(){revoked.store(true,Ordering::Release);break Err(Error::OutcomeUnknown)}
                        }}
                    }
                };
                let result = result.and_then(|(meta, binary)| meta.into_public(binary));
                let unknown = matches!(result, Err(Error::OutcomeUnknown | Error::Failed));
                if unknown {
                    revoked.store(true, Ordering::Release);
                    let _ = cleanup_owned(&mut child, &active_job).await;
                }
                let _ = reply.send(result);
            }
        }
    }
    closed.store(
        cleanup_owned(&mut child, &active_job).await.is_ok(),
        Ordering::Release,
    );
}

async fn cleanup_owned(
    child: &mut Option<Child>,
    active_job: &Mutex<Option<Arc<colossus_windows_native::KillOnCloseJob>>>,
) -> Result<(), Error> {
    let result = if let Some(process) = child.as_mut() {
        process.shutdown().await
    } else {
        Ok(())
    };
    commit_cleanup(child, result)?;
    active_job.lock().map_err(|_| Error::OutcomeUnknown)?.take();
    Ok(())
}
fn commit_cleanup<T>(owner: &mut Option<T>, result: Result<(), Error>) -> Result<(), Error> {
    result?;
    owner.take();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_cleanup_retains_exact_custody_until_a_later_confirmed_ack() {
        let mut child = Some(713u32);
        assert_eq!(
            commit_cleanup(&mut child, Err(Error::OutcomeUnknown)),
            Err(Error::OutcomeUnknown)
        );
        assert_eq!(child, Some(713));
        assert_eq!(
            commit_cleanup(&mut child, Err(Error::Failed)),
            Err(Error::Failed)
        );
        assert_eq!(child, Some(713));
        commit_cleanup(&mut child, Ok(())).unwrap();
        assert!(child.is_none());
        commit_cleanup(&mut child, Ok(())).unwrap();
        assert!(child.is_none());
    }
}
