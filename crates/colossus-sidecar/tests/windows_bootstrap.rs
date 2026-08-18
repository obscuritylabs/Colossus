//! Windows managed-sidecar process bootstrap acceptance.

#![cfg(windows)]

use colossus_windows_native::{
    BoundPath, KillOnCloseJob, configure_suspended_process, validate_named_pipe_client,
};
use std::process::Stdio;
use tokio::net::windows::named_pipe::ServerOptions;

const PIPE_ENVIRONMENT: &str = "COLOSSUS_WINDOWS_BOOTSTRAP_PIPE_V1";
const PARENT_ENVIRONMENT: &str = "COLOSSUS_WINDOWS_BOOTSTRAP_PARENT_PID_V1";

#[tokio::test]
async fn suspended_verified_sidecar_reaches_its_authenticated_bootstrap_pipe() {
    let executable = std::fs::canonicalize(env!("CARGO_BIN_EXE_colossus-sidecar"))
        .expect("canonical sidecar executable");
    let executable_binding = BoundPath::open_file(&executable).expect("bind sidecar executable");
    let pipe_name = format!(r"\\.\pipe\colossus-managed-{}", uuid::Uuid::now_v7());
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .access_inbound(true)
        .access_outbound(true)
        .create(&pipe_name)
        .expect("bootstrap named-pipe server");
    let instance = tempfile::tempdir().expect("sidecar instance directory");
    let mut command = tokio::process::Command::new(&executable);
    command
        .arg("__managed-sidecar-v1")
        .env_clear()
        .env(PIPE_ENVIRONMENT, &pipe_name)
        .env(PARENT_ENVIRONMENT, std::process::id().to_string())
        .current_dir(instance.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    configure_suspended_process(command.as_std_mut());
    let mut child = command.spawn().expect("spawn suspended sidecar");
    let (job, child_process) =
        KillOnCloseJob::assign_tokio_child_verify_and_resume(&child, executable_binding.identity())
            .expect("bind, verify, and resume sidecar");

    let connection =
        tokio::time::timeout(std::time::Duration::from_secs(5), server.connect()).await;
    if connection.is_err() {
        let status = child.wait().await.expect("wait for failed sidecar");
        panic!("sidecar bootstrap connection timeout; child status: {status}");
    }
    connection
        .expect("sidecar bootstrap connection timeout")
        .expect("sidecar bootstrap connection");
    validate_named_pipe_client(&server, child_process).expect("authenticate sidecar process");

    drop(server);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
        .await
        .expect("sidecar shutdown timeout")
        .expect("wait for sidecar");
    assert!(!status.success(), "EOF before bootstrap must fail closed");
    drop(job);
}
