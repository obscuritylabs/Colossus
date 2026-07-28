use crate::{FileIdentity, WindowsNativeError};
use std::os::windows::io::AsRawHandle;

/// A Windows Job Object configured to terminate the complete assigned process tree
/// when this owner is dropped.
pub struct KillOnCloseJob(crate::windows::KillOnCloseJob);

impl std::fmt::Debug for KillOnCloseJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KillOnCloseJob")
            .finish_non_exhaustive()
    }
}

impl KillOnCloseJob {
    /// Assign a suspended process to a kill-on-close Job Object, prove that its
    /// image still has the retained file identity, and resume its only thread.
    pub fn assign_verify_and_resume<P: AsRawHandle>(
        process: &P,
        process_id: u32,
        expected_image: FileIdentity,
    ) -> Result<Self, WindowsNativeError> {
        crate::windows::KillOnCloseJob::assign_verify_and_resume(
            process.as_raw_handle(),
            process_id,
            expected_image,
        )
        .map(Self)
    }

    /// Assign a Tokio-owned suspended process to a kill-on-close Job Object,
    /// prove that its image still has the retained file identity, and resume its
    /// only thread without exposing a raw Windows handle to the caller.
    pub fn assign_tokio_child_verify_and_resume(
        process: &tokio::process::Child,
        expected_image: FileIdentity,
    ) -> Result<(Self, u32), WindowsNativeError> {
        let process_id = process.id().ok_or(WindowsNativeError::InvalidInput)?;
        let raw_process = process
            .raw_handle()
            .ok_or(WindowsNativeError::InvalidInput)?;
        let job = crate::windows::KillOnCloseJob::assign_verify_and_resume(
            raw_process,
            process_id,
            expected_image,
        )
        .map(Self)?;
        Ok((job, process_id))
    }

    /// Terminate every process assigned to this job.
    pub fn terminate(&self) -> Result<(), WindowsNativeError> {
        self.0.terminate()
    }
}

/// Prove that a connected named-pipe server is talking to the exact spawned process.
pub fn validate_named_pipe_client<P: AsRawHandle>(
    pipe: &P,
    expected_process_id: u32,
) -> Result<(), WindowsNativeError> {
    crate::windows::validate_named_pipe_client(pipe.as_raw_handle(), expected_process_id)
}

/// Prove that a connected named-pipe client is talking to the expected parent process.
pub fn validate_named_pipe_server<P: AsRawHandle>(
    pipe: &P,
    expected_process_id: u32,
) -> Result<(), WindowsNativeError> {
    crate::windows::validate_named_pipe_server(pipe.as_raw_handle(), expected_process_id)
}

/// Point the current process standard input and output handles at one authenticated
/// duplex bootstrap pipe.
pub fn install_bootstrap_pipe_as_standard_io<P: AsRawHandle>(
    pipe: &P,
) -> Result<(), WindowsNativeError> {
    crate::windows::install_bootstrap_pipe_as_standard_io(pipe.as_raw_handle())
}

/// Configure a no-shell Windows command to start suspended and without a console
/// window so its image can be verified and job-bound before its first instruction.
pub fn configure_suspended_process(command: &mut std::process::Command) {
    crate::windows::configure_suspended_process(command);
}
