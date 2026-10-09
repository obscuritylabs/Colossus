use colossus_cloud_protocol::{
    CloudReply, Command, MAX_ACTIVE_TASKS, MAX_PAYLOAD_BYTES, PROTOCOL_MAJOR, RuntimeInventory,
    decode, encode,
    v1alpha1::{
        self as wire, control_frame, runtime_connection_client::RuntimeConnectionClient,
        runtime_frame,
    },
};
use colossus_sdk::{
    AgentRunClient, ApiErrorCode, CancelRunRequest, GetRunRequest, ListRunsRequest, PageRequest,
    WatchRunRequest,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::{
    sync::{Semaphore, watch},
    task::JoinSet,
};
use tonic::{
    Status,
    transport::{Certificate, ClientTlsConfig, Endpoint, Identity},
};
use zeroize::Zeroizing;

#[cfg(test)]
mod readiness_tests;
#[cfg(test)]
mod watcher_tests;

/// Non-secret persisted enrollment and local runtime identity binding.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    /// HTTPS cloud endpoint with independently enrolled CA trust.
    pub endpoint: String,
    /// Fixed project namespace.
    pub project_id: String,
    /// Stable cloud node identity.
    pub node_id: String,
    /// Independently verified local runtime instance.
    pub instance_id: String,
    /// PEM client leaf issued for this enrollment.
    pub certificate_pem: String,
    /// Independently obtained cloud TLS CA chain.
    pub ca_pem: String,
    /// Local runtime's authenticated optional capabilities.
    pub capabilities: Vec<String>,
    /// Native acknowledgement that this enrollment has been revoked.
    #[serde(default)]
    pub revoked: bool,
    /// Opaque host and workspace grouping, independent from authorization.
    #[serde(default)]
    pub inventory: Option<RuntimeInventory>,
    /// Local acknowledgement that explicitly shared sessions may be continued.
    #[serde(default)]
    pub shared_continuation: bool,
}
/// Sanitized lifecycle state, suitable for CLI and Desktop projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorStatus {
    /// Opening an outbound stream.
    Connecting,
    /// Authenticated cloud stream is live.
    Connected,
    /// Transport disappeared; fixed-node work continues locally.
    Reconnecting,
    /// Explicit disconnect completed without cancelling agent work.
    Disconnected,
    /// Cloud enrollment or client certificate was rejected; no retry occurs.
    Revoked,
}
/// Outbound connector bound to an independently authorized local application.
pub struct RuntimeConnector {
    config: ConnectionConfig,
    key: Zeroizing<String>,
    runs: Arc<dyn AgentRunClient>,
    enrollment: Option<crate::EnrollmentStore>,
    resources: crate::ConnectorResources,
}
impl RuntimeConnector {
    /// Bind native-held credentials and an SDK client carrying a dedicated cloud grant.
    /// No renderer credential or generic worker tunnel is accepted.
    pub fn new(
        config: ConnectionConfig,
        key: Zeroizing<String>,
        runs: Arc<dyn AgentRunClient>,
    ) -> Result<Self, &'static str> {
        if config.revoked {
            return Err("cloud enrollment was revoked");
        }
        let url = url::Url::parse(&config.endpoint).map_err(|_| "invalid cloud endpoint")?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err("cloud endpoint must be HTTPS");
        }
        if config.project_id.is_empty()
            || config.node_id.is_empty()
            || config.instance_id.is_empty()
            || key.len() > 16384
            || key.is_empty()
        {
            return Err("invalid connector enrollment");
        }
        if let Some(inventory) = &config.inventory {
            inventory
                .validate()
                .map_err(|_| "invalid connector inventory")?;
        }
        Ok(Self {
            config,
            key,
            runs,
            enrollment: None,
            resources: crate::ConnectorResources::default(),
        })
    }
    /// Attach only the dedicated cloud application's authenticated public resources.
    pub fn with_resources(mut self, resources: crate::ConnectorResources) -> Self {
        self.resources = resources;
        self
    }
    /// Retain the native enrollment authority for automatic, durable certificate renewal.
    pub fn with_enrollment_store(mut self, store: crate::EnrollmentStore) -> Self {
        self.enrollment = Some(store);
        self
    }
    /// Reconnect with bounded backoff until explicit shutdown or SDK lifetime closure. Dropping this future
    /// cancels watches only; it never cancels accepted runtime tasks or closes a daemon.
    pub async fn run(
        mut self,
        mut shutdown: watch::Receiver<bool>,
        status: watch::Sender<ConnectorStatus>,
    ) -> Result<(), &'static str> {
        let mut delay = 1u64;
        loop {
            if self.runs.is_closed() {
                status.send_replace(ConnectorStatus::Disconnected);
                return Ok(());
            }
            if *shutdown.borrow() {
                status.send_replace(ConnectorStatus::Disconnected);
                return Ok(());
            }
            // Reconcile a persisted rotation even when the old certificate is
            // still fresh: the host may have committed it before the ACK was lost.
            if let Some(store) = &self.enrollment {
                let renewed = tokio::select! {
                    result = store.renew(false) => result,
                    _ = shutdown.changed() => {
                        status.send_replace(ConnectorStatus::Disconnected);
                        return Ok(());
                    }
                };
                if let Err(error) = renewed {
                    if error == "cloud enrollment was revoked" {
                        status.send_replace(ConnectorStatus::Revoked);
                        return Err(error);
                    }
                    status.send_replace(ConnectorStatus::Reconnecting);
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(delay)) => {},
                        _ = shutdown.changed() => {
                            status.send_replace(ConnectorStatus::Disconnected);
                            return Ok(());
                        }
                    }
                    delay = (delay * 2).min(30);
                    continue;
                }
                let store = store.clone();
                let (config, key) = tokio::task::spawn_blocking(move || store.load())
                    .await
                    .map_err(|_| "enrollment vault unavailable")??
                    .ok_or("enrollment unavailable")?;
                self.config = config;
                self.key = key;
            }
            status.send_replace(if delay == 1 {
                ConnectorStatus::Connecting
            } else {
                ConnectorStatus::Reconnecting
            });
            // Keep child watches outside the cancellable connection future. A
            // cooperative shutdown must join their cancellation, not merely drop
            // a JoinSet whose child uploads may still be executing.
            let mut watchers = JoinSet::new();
            let result = tokio::select! {
                result = self.connect_once(&status, &mut watchers) => Some(result),
                _ = shutdown.changed() => None,
            };
            quiesce_watchers(&mut watchers).await;
            let Some(result) = result else {
                status.send_replace(ConnectorStatus::Disconnected);
                return Ok(());
            };
            if let Err(error) = &result {
                // Transport categories are diagnostic metadata. Never log server
                // messages, provider bodies, identifiers, or credential material.
                let reason = match error.message() {
                    "local runtime unavailable"
                    | "local runtime closed"
                    | "local readiness timeout" => "local readiness",
                    "local snapshot unavailable" => "local snapshot",
                    "local watch unavailable" => "local watch admission",
                    "local feed unavailable" => "local event feed",
                    "local discovery unavailable" | "local discovery timeout" => "local discovery",
                    "cloud storage unavailable" => "cloud storage",
                    _ => "cloud transport",
                };
                eprintln!("connector transport {:?} ({reason})", error.code());
            }
            if self.runs.is_closed() {
                status.send_replace(ConnectorStatus::Disconnected);
                return Ok(());
            }
            if result.as_ref().err().is_some_and(|error| {
                matches!(
                    error.code(),
                    tonic::Code::PermissionDenied | tonic::Code::Unauthenticated
                )
            }) {
                status.send_replace(ConnectorStatus::Revoked);
                return Err("cloud enrollment was rejected or revoked");
            }
            status.send_replace(ConnectorStatus::Reconnecting);
            tokio::select! {_=tokio::time::sleep(Duration::from_secs(delay))=>{},_=shutdown.changed()=>{status.send_replace(ConnectorStatus::Disconnected);return Ok(());}}
            delay = (delay * 2).min(30);
        }
    }
    async fn connect_once(
        &self,
        status: &watch::Sender<ConnectorStatus>,
        watchers: &mut JoinSet<Result<(), Status>>,
    ) -> Result<(), Status> {
        check_local_readiness(self.runs.as_ref()).await?;
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(self.config.ca_pem.clone()))
            .identity(Identity::from_pem(
                self.config.certificate_pem.clone(),
                self.key.as_bytes(),
            ));
        let endpoint = Endpoint::from_shared(self.config.endpoint.clone())
            .map_err(|_| Status::invalid_argument("invalid endpoint"))?
            .tls_config(tls)
            .map_err(|_| Status::invalid_argument("invalid TLS identity"))?
            .connect_timeout(Duration::from_secs(10));
        // Endpoint's connect_timeout covers TCP. Bound TLS and HTTP/2 startup too.
        let channel = tokio::time::timeout(Duration::from_secs(10), endpoint.connect())
            .await
            .map_err(|_| Status::deadline_exceeded("cloud channel startup timeout"))?
            .map_err(|_| Status::unavailable("cloud transport unavailable"))?;
        let mut client = RuntimeConnectionClient::new(channel)
            .max_decoding_message_size(MAX_PAYLOAD_BYTES + 16384)
            .max_encoding_message_size(MAX_PAYLOAD_BYTES + 16384);
        let (sender, receiver) = crate::outbound::channel();
        send(
            &sender,
            runtime_frame::Body::Hello(wire::RuntimeHello {
                protocol_major: PROTOCOL_MAJOR,
                node_id: self.config.node_id.clone(),
                instance_id: self.config.instance_id.clone(),
                capabilities: {
                    let mut capabilities = self.config.capabilities.clone();
                    capabilities.retain(|c| c != colossus_cloud_protocol::RESOURCE_CAPABILITY);
                    capabilities.push(colossus_cloud_protocol::RESOURCE_CAPABILITY.into());
                    capabilities
                },
                project_id: self.config.project_id.clone(),
                inventory_json: self
                    .config
                    .inventory
                    .as_ref()
                    .map(|inventory| {
                        let mut legacy = inventory.clone();
                        legacy.policy = None;
                        encode(&legacy)
                    })
                    .transpose()
                    .map_err(|_| Status::invalid_argument("invalid native inventory"))?
                    .unwrap_or_default(),
            }),
        )
        .await?;
        let mut stream = tokio::time::timeout(Duration::from_secs(10), client.connect(receiver))
            .await
            .map_err(|_| Status::deadline_exceeded("cloud response headers timeout"))??
            .into_inner();
        let welcome = tokio::time::timeout(Duration::from_secs(10), stream.message())
            .await
            .map_err(|_| Status::deadline_exceeded("cloud welcome timeout"))??
            .ok_or_else(|| Status::unavailable("cloud closed"))?;
        if !matches!(welcome.body,Some(control_frame::Body::Welcome(ref welcome)) if welcome.protocol_major==PROTOCOL_MAJOR)
        {
            return Err(Status::failed_precondition("cloud protocol mismatch"));
        }
        status.send_replace(ConnectorStatus::Connected);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let permits = Arc::new(Semaphore::new(MAX_ACTIVE_TASKS));
        let resource_permits = Arc::new(Semaphore::new(
            colossus_cloud_protocol::MAX_RESOURCE_REQUESTS,
        ));
        let mut watched = HashSet::new();
        let mut discovery: Option<wire::ReleasedDiscoveryPage> = None;
        let mut acknowledged_discovery: Option<(String, String)> = None;
        let mut policy_refresh = tokio::time::Instant::now() - Duration::from_secs(15);
        loop {
            tokio::select! {
                _=heartbeat.tick()=>{
                    check_local_readiness(self.runs.as_ref()).await?;
                    if self.enrollment.is_some() && crate::enrollment::certificate_due(&self.config.certificate_pem).map_err(|_|Status::failed_precondition("invalid connector certificate"))? { return Ok(()); }
                    let mut inventory_json = Vec::new();
                    if policy_refresh.elapsed() >= Duration::from_secs(15) {
                        policy_refresh = tokio::time::Instant::now();
                        if let Some(mut inventory) = self.config.inventory.clone()
                            && let Ok(Ok(policy)) = tokio::time::timeout(Duration::from_secs(2), self.runs.get_runtime_policy_posture()).await {
                            inventory.policy = Some(policy);
                            inventory_json = encode(&inventory).map_err(|_| Status::failed_precondition("invalid native inventory"))?;
                        }
                    }
                    send(&sender,runtime_frame::Body::Heartbeat(wire::RuntimeHeartbeat{ready:true,inventory_json})).await?;
                },
                finished=watchers.join_next(),if !watchers.is_empty()=>{
                    finished.ok_or_else(||Status::internal("watch lost"))?.map_err(|_|Status::internal("watch failed"))??;
                }
                frame=stream.message()=>{
                    let Some(frame)=frame? else{return Ok(());};
                    match frame.body {
                        Some(control_frame::Body::Command(command))=>{
                            let operation:Command=decode(&command.command_json).map_err(|_|Status::invalid_argument("invalid cloud command"))?;
                            if let Command::Watch{run_id,snapshot_only}=operation {
                                if watched.contains(&command.task_id){continue;}
                                let permit=permits.clone().try_acquire_owned().map_err(|_|Status::resource_exhausted("local watch bound"))?;
                                if watched.len()>=4096{return Err(Status::resource_exhausted("connection task bound"));}
                                watched.insert(command.task_id.clone());
                                let runs=self.runs.clone();let sender=sender.clone();
                                watchers.spawn(async move{let _permit=permit;watch_task(runs,sender,command.task_id,run_id,command.after_sequence,snapshot_only).await});
                            } else {
                                let mut reply=execute(self.runs.as_ref(),operation).await;
                                // An unavailable reply can follow an accepted mutation. Reconnect
                                // with its stable runtime idempotency key to reconcile it.
                                if matches!(&reply,CloudReply::Failed{error} if error.code==ApiErrorCode::Unavailable){return Err(Status::unavailable("local runtime unavailable"));}
                                if let CloudReply::Cancelled { response } = &mut reply {
                                    crate::released::compact_cancellation(response).map_err(Status::resource_exhausted)?;
                                }
                                let limited_run = if let CloudReply::Run {run} = &mut reply {
                                    crate::released::compact_snapshot(run).map_err(Status::resource_exhausted)?.then(||run.run.run_id.clone())
                                } else {None};
                                send(&sender,runtime_frame::Body::Receipt(wire::CommandReceipt{command_id:command.command_id,task_id:command.task_id.clone(),reply_json:encode(&reply).map_err(|_|Status::resource_exhausted("receipt too large"))?})).await?;
                                if let Some(run_id) = limited_run {
                                    send(&sender,runtime_frame::Body::OutputLimit(wire::ReleasedOutputLimit{task_id:command.task_id, run_id, after_sequence:0})).await?;
                                }
                            }
                        }
                        Some(control_frame::Body::ResourceRequest(request)) => {
                            if request.request_id.len() != 32 || !request.request_id.bytes().all(|b| b.is_ascii_hexdigit()) || request.operation_json.len() > colossus_cloud_protocol::MAX_RESOURCE_REQUEST_BYTES {
                                return Err(Status::invalid_argument("invalid resource request"));
                            }
                            let operation: colossus_cloud_protocol::ResourceOperation = decode(&request.operation_json).map_err(|_| Status::invalid_argument("invalid resource operation"))?;
                            let permit = resource_permits.clone().try_acquire_owned().map_err(|_| Status::resource_exhausted("resource request bound"))?;
                            let resources = self.resources.clone(); let sender = sender.clone();
                            watchers.spawn(async move {
                                let _permit = permit;
                                let reply = resources.execute(operation).await;
                                send(&sender, runtime_frame::Body::ResourceResponse(wire::ResourceResponse { request_id: request.request_id, reply_json: encode(&reply).map_err(|_| Status::resource_exhausted("resource response too large"))? })).await
                            });
                        },
                        Some(control_frame::Body::Acknowledgement(_))=>{},
                        Some(control_frame::Body::Discover(request))=>{
                            if let Some(page) = &discovery {
                                if page.sync_id != request.sync_id || page.page_token != request.page_token {
                                    return Err(Status::failed_precondition("discovery page not acknowledged"));
                                }
                                send(&sender, runtime_frame::Body::Discovery(page.clone())).await?;
                            } else {
                                let page = crate::discovery::discover(self.runs.as_ref(), request).await?;
                                send(&sender, runtime_frame::Body::Discovery(page.clone())).await?;
                                discovery = Some(page);
                            }
                        },
                        Some(control_frame::Body::DiscoveryAcknowledgement(ack))=>{
                            if discovery.as_ref().is_some_and(|page|page.sync_id == ack.sync_id && page.page_token == ack.page_token) {
                                discovery = None;
                                acknowledged_discovery = Some((ack.sync_id,ack.page_token));
                            } else if !acknowledged_discovery.as_ref().is_some_and(|(sync_id,page_token)|sync_id == &ack.sync_id && page_token == &ack.page_token) {
                                return Err(Status::invalid_argument("unexpected discovery acknowledgement"));
                            }
                        },
                        _=>return Err(Status::invalid_argument("unexpected cloud frame")),
                    }
                }
            }
        }
    }
}

// An external daemon's SDK client stays open across daemon outages. Probe its
// authenticated, caller-scoped read API before advertising execution readiness.
// The one-item page is discarded locally and never forwarded to the cloud.
async fn check_local_readiness(runs: &dyn AgentRunClient) -> Result<(), Status> {
    if runs.is_closed() {
        return Err(Status::unavailable("local runtime closed"));
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let result = runs
                .list_runs(ListRunsRequest {
                    session_id: None,
                    statuses: Vec::new(),
                    page: Some(PageRequest {
                        page_size: 1,
                        page_token: String::new(),
                    }),
                    include_archived: false,
                })
                .await;
            match result {
                Ok(_) => return Ok(()),
                Err(error) if error.code == ApiErrorCode::ResourceExhausted => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(error) => return Err(local_read_error(error, "local runtime unavailable")),
            }
        }
    })
    .await
    .map_err(|_| Status::deadline_exceeded("local readiness timeout"))??;
    if runs.is_closed() {
        return Err(Status::unavailable("local runtime closed"));
    }
    Ok(())
}
fn local_read_error(error: colossus_sdk::ApiError, message: &'static str) -> Status {
    match error.code {
        ApiErrorCode::ResourceExhausted => Status::resource_exhausted(message),
        ApiErrorCode::Unavailable => Status::unavailable(message),
        ApiErrorCode::PermissionDenied | ApiErrorCode::Unauthenticated => {
            Status::failed_precondition("local SDK grant rejected")
        }
        _ => Status::failed_precondition(message),
    }
}
async fn execute(runs: &dyn AgentRunClient, command: Command) -> CloudReply {
    let result = match command {
        Command::InspectInboxes {
            root_run_id,
            participant_id,
            after_sequence,
        } => crate::inboxes::read(runs, root_run_id, participant_id, after_sequence).await,
        Command::History {
            source_run_id,
            page_token,
            page_size,
        } => crate::history::read(runs, source_run_id, page_token, page_size).await,
        Command::Create { request } => match runs.create_run(*request).await {
            Ok(created) => runs
                .get_run(GetRunRequest {
                    run_id: created.run.run_id,
                })
                .await
                .map(|run| CloudReply::Run { run: Box::new(run) }),
            Err(error) => Err(error),
        },
        Command::Cancel {
            run_id,
            idempotency_key,
        } => runs
            .cancel_run(CancelRunRequest {
                run_id,
                idempotency_key,
            })
            .await
            .map(|response| CloudReply::Cancelled { response }),
        Command::Respond { request } => runs
            .respond_interaction(*request)
            .await
            .map(|response| CloudReply::Responded { response }),
        Command::Watch { .. } => unreachable!("watch dispatch is handled separately"),
    };
    result.unwrap_or_else(|error| CloudReply::Failed { error })
}
async fn watch_task(
    runs: Arc<dyn AgentRunClient>,
    sender: crate::outbound::FrameSender,
    task_id: String,
    run_id: String,
    after: u64,
    snapshot_only: bool,
) -> Result<(), Status> {
    let mut snapshot = runs
        .get_run(GetRunRequest {
            run_id: run_id.clone(),
        })
        .await
        .map_err(|error| local_read_error(error, "local snapshot unavailable"))?;
    let limited =
        crate::released::compact_snapshot(&mut snapshot).map_err(Status::resource_exhausted)?;
    send(
        &sender,
        runtime_frame::Body::Snapshot(wire::RuntimeSnapshot {
            task_id: task_id.clone(),
            snapshot_json: encode(&snapshot)
                .map_err(|_| Status::resource_exhausted("snapshot bound"))?,
        }),
    )
    .await?;
    if limited && !snapshot_only {
        send(
            &sender,
            runtime_frame::Body::OutputLimit(wire::ReleasedOutputLimit {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                after_sequence: after,
            }),
        )
        .await?;
        return Ok(());
    }
    if snapshot_only {
        let mut snapshot = snapshot;
        loop {
            if snapshot.run.terminal.is_some() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            snapshot = runs
                .get_run(GetRunRequest {
                    run_id: run_id.clone(),
                })
                .await
                .map_err(|error| local_read_error(error, "local snapshot unavailable"))?;
            crate::released::compact_snapshot(&mut snapshot).map_err(Status::resource_exhausted)?;
            send(
                &sender,
                runtime_frame::Body::Snapshot(wire::RuntimeSnapshot {
                    task_id: task_id.clone(),
                    snapshot_json: encode(&snapshot)
                        .map_err(|_| Status::resource_exhausted("snapshot bound"))?,
                }),
            )
            .await?;
        }
    }
    let mut stream = runs
        .watch_run(WatchRunRequest {
            run_id: run_id.clone(),
            after_sequence: after,
        })
        .await
        .map_err(|error| local_read_error(error, "local watch unavailable"))?;
    while let Some(update) = stream.next().await {
        let update = update.map_err(|error| local_read_error(error, "local feed unavailable"))?;
        let update_json = match encode(&update) {
            Ok(bytes) => bytes,
            Err(_) => {
                send(
                    &sender,
                    runtime_frame::Body::OutputLimit(wire::ReleasedOutputLimit {
                        task_id: task_id.clone(),
                        run_id: run_id.clone(),
                        after_sequence: update.sequence.saturating_sub(1),
                    }),
                )
                .await?;
                // Let the host durably record the limit and close this stream.
                // Closing locally here could discard the queued limit frame.
                return Ok(());
            }
        };
        send(
            &sender,
            runtime_frame::Body::Event(wire::ReleasedRunEvent {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                sequence: update.sequence,
                update_json,
            }),
        )
        .await?;
        let mut snapshot = runs
            .get_run(GetRunRequest {
                run_id: run_id.clone(),
            })
            .await
            .map_err(|error| local_read_error(error, "local snapshot unavailable"))?;
        let limited =
            crate::released::compact_snapshot(&mut snapshot).map_err(Status::resource_exhausted)?;
        send(
            &sender,
            runtime_frame::Body::Snapshot(wire::RuntimeSnapshot {
                task_id: task_id.clone(),
                snapshot_json: encode(&snapshot)
                    .map_err(|_| Status::resource_exhausted("snapshot bound"))?,
            }),
        )
        .await?;
        if limited {
            send(
                &sender,
                runtime_frame::Body::OutputLimit(wire::ReleasedOutputLimit {
                    task_id: task_id.clone(),
                    run_id: run_id.clone(),
                    after_sequence: update.sequence,
                }),
            )
            .await?;
            return Ok(());
        }
    }
    Ok(())
}
async fn send(
    sender: &crate::outbound::FrameSender,
    body: runtime_frame::Body,
) -> Result<(), Status> {
    tokio::time::timeout(
        Duration::from_secs(10),
        sender.send(wire::RuntimeFrame { body: Some(body) }),
    )
    .await
    .map_err(|_| Status::resource_exhausted("outbound backpressure deadline"))?
    .map_err(|_| Status::cancelled("cloud stream closed"))
}

async fn quiesce_watchers(watchers: &mut JoinSet<Result<(), Status>>) {
    // These tasks own released-output watches, never the accepted local run.
    watchers.shutdown().await;
}
