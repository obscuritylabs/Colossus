use crate::server::{NodePresence, State, db};
use colossus_cloud::{CloudError, CloudNode};
use colossus_cloud_protocol::{
    Command, MAX_PAYLOAD_BYTES, MAX_QUEUED_FRAMES, PROTOCOL_MAJOR, decode, decode_update, encode,
    v1alpha1::{
        self as wire, control_frame,
        runtime_connection_server::{RuntimeConnection, RuntimeConnectionServer},
        runtime_frame,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    Request, Response, Status, Streaming,
    transport::server::{TcpConnectInfo, TlsConnectInfo},
};

pub(crate) fn service(state: Arc<State>) -> RuntimeConnectionServer<Connection> {
    RuntimeConnectionServer::new(Connection(state))
        .max_decoding_message_size(MAX_PAYLOAD_BYTES + 16384)
        .max_encoding_message_size(MAX_PAYLOAD_BYTES + 16384)
}
pub(crate) struct Connection(Arc<State>);
#[tonic::async_trait]
impl RuntimeConnection for Connection {
    type ConnectStream = ReceiverStream<Result<wire::ControlFrame, Status>>;
    async fn revoke_enrollment(
        &self,
        request: Request<wire::EnrollmentIdentity>,
    ) -> Result<Response<wire::EnrollmentRevoked>, Status> {
        let fingerprint = peer_fingerprint(&request)?;
        let input = request.into_inner();
        db(self.0.repo.clone(), move |repo| async move {
            repo.revoke_own_node(
                &input.project_id,
                &input.node_id,
                &fingerprint,
                &input.instance_id,
            )
            .await
        })
        .await
        .map_err(status)?;
        Ok(Response::new(wire::EnrollmentRevoked {}))
    }
    async fn renew_certificate(
        &self,
        request: Request<wire::CertificateRenewal>,
    ) -> Result<Response<wire::RenewedCertificate>, Status> {
        let fingerprint = peer_fingerprint(&request)?;
        let input = request.into_inner();
        let (pem, fingerprint_new) = self
            .0
            .ca
            .sign(&input.csr_pem)
            .map_err(|_| Status::invalid_argument("invalid certificate request"))?;
        let certificate = colossus_cloud::CertificateRedemption {
            fingerprint: fingerprint_new,
            csr_sha256: hex::encode(Sha256::digest(input.csr_pem.as_bytes())),
            certificate_pem: pem,
        };
        let certificate_pem = db(self.0.repo.clone(), move |repo| async move {
            repo.renew_certificate(
                colossus_cloud::RenewalIdentity {
                    project_id: &input.project_id,
                    node_id: &input.node_id,
                    instance_id: &input.instance_id,
                    previous_fingerprint: &fingerprint,
                    renewal_id: &input.renewal_id,
                },
                certificate,
                crate::http::now(),
            )
            .await
        })
        .await
        .map_err(status)?;
        Ok(Response::new(wire::RenewedCertificate { certificate_pem }))
    }
    async fn connect(
        &self,
        request: Request<Streaming<wire::RuntimeFrame>>,
    ) -> Result<Response<Self::ConnectStream>, Status> {
        let fingerprint = peer_fingerprint(&request)?;
        let mut inbound = request.into_inner();
        let first = tokio::time::timeout(Duration::from_secs(10), inbound.message())
            .await
            .map_err(|_| Status::deadline_exceeded("hello timeout"))??
            .ok_or_else(|| Status::invalid_argument("hello required"))?;
        let Some(runtime_frame::Body::Hello(hello)) = first.body else {
            return Err(Status::invalid_argument("hello required"));
        };
        if hello.protocol_major != PROTOCOL_MAJOR
            || hello.capabilities.len() > 64
            || hello
                .capabilities
                .iter()
                .any(|cap| cap.is_empty() || cap.len() > 128 || cap.chars().any(char::is_control))
        {
            return Err(Status::failed_precondition(
                "incompatible protocol or capabilities",
            ));
        }
        let node = db(self.0.repo.clone(), {
            let hello = hello.clone();
            move |repo| async move {
                repo.authenticate_node(
                    &hello.project_id,
                    &hello.node_id,
                    &fingerprint,
                    &hello.instance_id,
                )
                .await
            }
        })
        .await
        .map_err(status)?;
        let inventory = if hello.inventory_json.is_empty() {
            None
        } else {
            let inventory: colossus_cloud_protocol::RuntimeInventory =
                decode(&hello.inventory_json)
                    .map_err(|_| Status::invalid_argument("invalid runtime inventory"))?;
            inventory
                .validate()
                .map_err(|_| Status::invalid_argument("invalid runtime inventory"))?;
            Some(inventory)
        };
        let discovery_enabled = inventory.is_some();
        let connection_id = format!("{}-{}", self.0.replica_id, uuid::Uuid::now_v7().simple());
        let key = format!("{}:{}", node.project_id, node.node_id);
        let lease = self
            .0
            .repo
            .storage()
            .claim_lease(
                &node.project_id,
                &node.node_id,
                &connection_id,
                crate::http::now(),
                30,
            )
            .await
            .map_err(status)?;
        let prepared = async {
            let node = if let Some(inventory) = inventory {
                tokio::time::timeout(
                    Duration::from_secs(5),
                    self.0.repo.with_lease(lease.clone()).register_inventory(
                        &node,
                        inventory,
                        crate::http::now(),
                    ),
                )
                .await
                .map_err(|_| Status::deadline_exceeded("inventory initialization timeout"))?
                .map_err(status)?
            } else {
                node
            };
            {
                let mut presence = self
                    .0
                    .presence
                    .lock()
                    .map_err(|_| Status::unavailable("presence unavailable"))?;
                // The committed claim already fenced any expired previous owner.
                if presence.len() >= 1024 && !presence.contains_key(&key) {
                    return Err(Status::resource_exhausted("node connection limit"));
                }
                presence.insert(
                    key.clone(),
                    NodePresence {
                        connection_id: connection_id.clone(),
                        ready: false,
                        capabilities: hello.capabilities,
                        heartbeat: Some(Instant::now()),
                    },
                );
            }
            let (sender, receiver) = mpsc::channel(MAX_QUEUED_FRAMES);
            sender
                .send(Ok(wire::ControlFrame {
                    body: Some(control_frame::Body::Welcome(wire::ConnectionWelcome {
                        protocol_major: PROTOCOL_MAJOR,
                        connection_id: connection_id.clone(),
                    })),
                }))
                .await
                .map_err(|_| Status::cancelled("connection closed"))?;
            Ok::<_, Status>((node, sender, receiver))
        }
        .await;
        let (node, sender, receiver) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = self.0.repo.storage().release_lease(&lease).await;
                if let Ok(mut presence) = self.0.presence.lock()
                    && presence
                        .get(&key)
                        .is_some_and(|entry| entry.connection_id == connection_id)
                {
                    presence.remove(&key);
                }
                return Err(error);
            }
        };
        let state = self.0.clone();
        tokio::spawn(async move {
            let result = serve_connection(
                state.clone(),
                ConnectionScope {
                    node,
                    key: key.clone(),
                    generation: connection_id.clone(),
                    lease: lease.clone(),
                    discovery_enabled,
                },
                inbound,
                &sender,
            )
            .await;
            let _ = state.repo.storage().release_lease(&lease).await;
            if let Ok(mut presence) = state.presence.lock()
                && presence
                    .get(&key)
                    .is_some_and(|current| current.connection_id == connection_id)
            {
                presence.remove(&key);
            }
            if let Err(error) = result {
                eprintln!("Runtime connection ended: {}", error.code());
                let _ = sender.send(Err(error)).await;
            }
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }
}
fn peer_fingerprint<T>(request: &Request<T>) -> Result<String, Status> {
    let peer = request
        .extensions()
        .get::<TlsConnectInfo<TcpConnectInfo>>()
        .and_then(TlsConnectInfo::peer_certs)
        .and_then(|certificates| certificates.first().cloned())
        .ok_or_else(|| Status::unauthenticated("client certificate required"))?;
    Ok(hex::encode(Sha256::digest(peer.as_ref())))
}
struct ConnectionScope {
    node: CloudNode,
    key: String,
    generation: String,
    lease: colossus_cloud::storage::ConnectionLease,
    discovery_enabled: bool,
}
async fn serve_connection(
    state: Arc<State>,
    scope: ConnectionScope,
    mut inbound: Streaming<wire::RuntimeFrame>,
    sender: &mpsc::Sender<Result<wire::ControlFrame, Status>>,
) -> Result<(), Status> {
    let ConnectionScope {
        node,
        key,
        generation,
        mut lease,
        discovery_enabled,
    } = scope;
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_heartbeat = Instant::now();
    let mut sent = HashSet::new();
    let mut watched = HashSet::new();
    let mut command_cursor: Option<String> = None;
    let mut task_cursor: Option<String> = None;
    let mut shutdown = state.shutdown.subscribe();
    let mut hints = state.repo.storage().subscribe();
    let mut discovery: Option<(String, String)> = None;
    let mut last_discovery = Instant::now() - Duration::from_secs(30);
    let mut visible_sessions = std::collections::BTreeMap::<String, bool>::new();
    let mut last_presence = Instant::now();
    let mut last_dispatch = Instant::now() - Duration::from_secs(1);
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let repo = state.repo.with_lease(lease.clone());
        tokio::select! {
            _=shutdown.changed()=>return Ok(()),
            frame=inbound.message()=>{
                let frame=frame?.ok_or_else(||Status::cancelled("runtime disconnected"))?;
                state.repo.storage().verify_lease(&lease,crate::http::now()).await.map_err(status)?;
                match frame.body {
                    Some(runtime_frame::Body::Heartbeat(heartbeat))=>{
                        repo.authenticate_node(&node.project_id,&node.node_id,&node.certificate_sha256,&node.instance_id).await.map_err(status)?;
                        lease=state.repo.storage().renew_lease(&lease,crate::http::now(),30).await.map_err(status)?;
                        if !heartbeat.inventory_json.is_empty() {
                            let inventory:colossus_cloud_protocol::RuntimeInventory=decode(&heartbeat.inventory_json).map_err(|_|Status::invalid_argument("invalid runtime inventory"))?;
                            repo.register_inventory(&node,inventory,crate::http::now()).await.map_err(status)?;
                            repo.heartbeat_node(&node,heartbeat.ready,crate::http::now()).await.map_err(status)?;
                            last_presence=Instant::now();
                        }
                        last_heartbeat=Instant::now();
                        {let mut presence=state.presence.lock().map_err(|_|Status::unavailable("presence unavailable"))?;
                        let current=presence.get_mut(&key).filter(|current|current.connection_id==generation).ok_or_else(||Status::aborted("connection superseded"))?;
                        current.ready=heartbeat.ready;current.heartbeat=Some(last_heartbeat);}
                        if last_presence.elapsed()>Duration::from_secs(10){repo.heartbeat_node(&node,heartbeat.ready,crate::http::now()).await.map_err(status)?;last_presence=Instant::now();}
                    },
                    Some(runtime_frame::Body::Receipt(receipt))=>{
                        let reply=decode(&receipt.reply_json).map_err(|_|Status::invalid_argument("invalid receipt"))?;
                        repo.record_receipt(&node,&receipt.command_id,&receipt.task_id,reply).await.map_err(status)?;
                    },
                    Some(runtime_frame::Body::Snapshot(snapshot))=>{
                        let snapshot_value=decode(&snapshot.snapshot_json).map_err(|_|Status::invalid_argument("invalid snapshot"))?;
                        repo.record_snapshot(&node,&snapshot.task_id,snapshot_value).await.map_err(status)?;
                    },
                    Some(runtime_frame::Body::Event(event))=>{
                        let update=decode_update(&event.update_json,&event.run_id,event.sequence).map_err(|_|Status::invalid_argument("invalid released event"))?;
                        let sequence=repo.record_update(&node,&event.task_id,update).await.map_err(status)?;
                        send(sender,control_frame::Body::Acknowledgement(wire::EventAcknowledgement{task_id:event.task_id,sequence})).await?;
                    },
                    Some(runtime_frame::Body::Discovery(page))=>{
                        let Some((sync_id,page_token))=&discovery else{return Err(Status::invalid_argument("discovery was not requested"));};
                        if page.sync_id!=*sync_id || page.page_token!=*page_token{return Err(Status::invalid_argument("discovery cursor does not match"));}
                        let runs:Vec<colossus_cloud_protocol::ReleasedRunInventory>=decode(&page.runs_json).map_err(|_|Status::invalid_argument("invalid discovery page"))?;
                        for entry in &runs {
                            visible_sessions.entry(entry.run.session_id.clone()).and_modify(|allowed|*allowed|=entry.continuable).or_insert(entry.continuable);
                            if visible_sessions.len()>16_384 {return Err(Status::resource_exhausted("workspace discovery session bound"));}
                        }
                        repo.discover_runs(&node,runs,&page.sync_id).await.map_err(status)?;
                        // ACK is emitted only after all idempotent source mappings commit.
                        send(sender,control_frame::Body::DiscoveryAcknowledgement(wire::DiscoveryAcknowledgement{sync_id:page.sync_id.clone(),page_token:page.page_token})).await?;
                        if page.next_page_token.is_empty(){repo.finish_discovery(&node,&visible_sessions).await.map_err(status)?;discovery=None;last_discovery=Instant::now();visible_sessions.clear();}else{
                            discovery=Some((page.sync_id.clone(),page.next_page_token.clone()));
                            send(sender,control_frame::Body::Discover(wire::DiscoverRuns{sync_id:page.sync_id,page_token:page.next_page_token,page_size:colossus_cloud_protocol::MAX_DISCOVERY_PAGE_SIZE,include_archived:true})).await?;
                        }
                    },
                    Some(runtime_frame::Body::OutputLimit(limit))=>{
                        repo.record_output_limit(&node,&limit.task_id,&limit.run_id,limit.after_sequence).await.map_err(status)?;
                        return Err(Status::resource_exhausted("released output retention limit"));
                    },
                    _=>return Err(Status::invalid_argument("unexpected runtime frame")),
                }
            },
            _=interval.tick()=>{},
            _=hints.recv()=>{},
        }
        if last_dispatch.elapsed() < Duration::from_millis(500) {
            continue;
        }
        last_dispatch = Instant::now();
        if last_heartbeat.elapsed() > Duration::from_secs(30) {
            return Err(Status::deadline_exceeded("heartbeat expired"));
        }
        state
            .repo
            .storage()
            .verify_lease(&lease, crate::http::now())
            .await
            .map_err(status)?;
        if discovery_enabled
            && discovery.is_none()
            && last_discovery.elapsed() >= Duration::from_secs(15)
        {
            let sync_id = uuid::Uuid::now_v7().simple().to_string();
            discovery = Some((sync_id.clone(), String::new()));
            send(
                sender,
                control_frame::Body::Discover(wire::DiscoverRuns {
                    sync_id,
                    page_token: String::new(),
                    page_size: colossus_cloud_protocol::MAX_DISCOVERY_PAGE_SIZE,
                    include_archived: true,
                }),
            )
            .await?;
        }
        let commands = repo
            .commands(&node, command_cursor.as_deref(), 100)
            .await
            .map_err(status)?;
        command_cursor = if commands.len() == 100 {
            commands.last().map(|command| command.command_id.clone())
        } else {
            None
        };
        for command in commands {
            if command.reply.is_none()
                && !sent.contains(&command.command_id)
                && repo
                    .command_dispatchable(&node, &command)
                    .await
                    .map_err(status)?
            {
                if sent.len() >= 4096 {
                    return Err(Status::resource_exhausted("connection command limit"));
                }
                let bytes = encode(&command.command)
                    .map_err(|_| Status::invalid_argument("invalid command"))?;
                state
                    .repo
                    .storage()
                    .verify_lease(&lease, crate::http::now())
                    .await
                    .map_err(status)?;
                send(
                    sender,
                    control_frame::Body::Command(wire::CloudCommand {
                        command_id: command.command_id.clone(),
                        task_id: command.task_id,
                        command_json: bytes,
                        after_sequence: 0,
                    }),
                )
                .await?;
                sent.insert(command.command_id);
            }
        }
        let tasks = repo
            .node_tasks(&node, task_cursor.as_deref(), 100)
            .await
            .map_err(status)?;
        task_cursor = if tasks.len() == 100 {
            tasks.last().map(|task| task.task_id.clone())
        } else {
            None
        };
        for task in tasks {
            let Some(run_id) = task.run_id else {
                continue;
            };
            let complete = task.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.run.terminal.is_some()
                    && (task.output_limited || task.last_sequence >= snapshot.run.last_sequence)
            });
            if complete {
                watched.remove(&task.task_id);
                continue;
            }
            if !watched.contains(&task.task_id)
                && watched.len() < colossus_cloud_protocol::MAX_ACTIVE_TASKS.min(8)
            {
                let bytes = encode(&Command::Watch {
                    run_id,
                    snapshot_only: task.output_limited,
                })
                .map_err(|_| Status::invalid_argument("invalid watch"))?;
                state
                    .repo
                    .storage()
                    .verify_lease(&lease, crate::http::now())
                    .await
                    .map_err(status)?;
                send(
                    sender,
                    control_frame::Body::Command(wire::CloudCommand {
                        command_id: format!("watch-{}", task.task_id),
                        task_id: task.task_id.clone(),
                        command_json: bytes,
                        after_sequence: task.last_sequence,
                    }),
                )
                .await?;
                watched.insert(task.task_id);
            }
        }
    }
}
async fn send(
    sender: &mpsc::Sender<Result<wire::ControlFrame, Status>>,
    body: control_frame::Body,
) -> Result<(), Status> {
    tokio::time::timeout(
        Duration::from_secs(10),
        sender.send(Ok(wire::ControlFrame { body: Some(body) })),
    )
    .await
    .map_err(|_| Status::resource_exhausted("connection backpressure deadline"))?
    .map_err(|_| Status::cancelled("connection closed"))
}
fn status(error: CloudError) -> Status {
    match error {
        CloudError::PermissionDenied => {
            Status::permission_denied("enrollment revoked or authority absent")
        }
        CloudError::InvalidArgument => Status::invalid_argument("invalid cloud frame"),
        CloudError::NotFound => Status::not_found("allocation not found"),
        CloudError::Conflict => Status::aborted("durable reconciliation conflict"),
        CloudError::ResourceExhausted => Status::resource_exhausted("cloud resource bound"),
        CloudError::Storage => Status::unavailable("cloud storage unavailable"),
    }
}
