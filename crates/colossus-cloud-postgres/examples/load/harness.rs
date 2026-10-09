use colossus_cloud::{CloudCaller, CloudNode, CloudPermission, CloudRepository, storage::*};
use colossus_cloud_postgres::{CloudDatabaseConfig, CloudDatabaseTls, CloudPostgresStore};
use colossus_cloud_protocol::CloudReply;
use colossus_network::AdditionalRootCertificates;
use colossus_sdk::*;
use futures::{StreamExt, stream};
use serde::Serialize;
use serde_json::json;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const PROJECT: &str = "load-project";

#[derive(Clone)]
struct Options {
    streams: usize,
    rate: usize,
    seconds: usize,
    pool: u32,
    payload: usize,
    keep: bool,
    report: Option<PathBuf>,
}
fn options() -> Result<Options, &'static str> {
    let mut options = Options {
        streams: 300,
        rate: 1000,
        seconds: 20,
        pool: 16,
        payload: 512,
        keep: false,
        report: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--streams" => {
                options.streams = args
                    .next()
                    .ok_or("streams required")?
                    .parse()
                    .map_err(|_| "invalid streams")?
            }
            "--rate" => {
                options.rate = args
                    .next()
                    .ok_or("rate required")?
                    .parse()
                    .map_err(|_| "invalid rate")?
            }
            "--seconds" => {
                options.seconds = args
                    .next()
                    .ok_or("seconds required")?
                    .parse()
                    .map_err(|_| "invalid seconds")?
            }
            "--pool" => {
                options.pool = args
                    .next()
                    .ok_or("pool required")?
                    .parse()
                    .map_err(|_| "invalid pool")?
            }
            "--payload-bytes" => {
                options.payload = args
                    .next()
                    .ok_or("payload required")?
                    .parse()
                    .map_err(|_| "invalid payload")?
            }
            "--keep" => options.keep = true,
            "--report" => options.report = Some(args.next().ok_or("report path required")?.into()),
            _ => return Err("unknown option"),
        }
    }
    if !(1..=1000).contains(&options.streams)
        || !(1..=10000).contains(&options.rate)
        || !(1..=120).contains(&options.seconds)
        || !(1..=128).contains(&options.pool)
        || !(64..=8192).contains(&options.payload)
        || options.rate * options.seconds > 250_000
    {
        return Err("measurement bounds exceeded");
    }
    Ok(options)
}
fn timestamp() -> Result<String, &'static str> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| "clock format failed")
}
fn now() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "clock unavailable")
}
fn request(index: usize) -> CreateRunRequest {
    CreateRunRequest {
        plugin_skill_ids: vec![],
        input: vec![InputContentPart::Text(format!(
            "Load fixture conversation {index}"
        ))],
        session_id: None,
        end_user_id: None,
        role: "primary".into(),
        mode: RunMode::Execute,
        research_depth: None,
        research_sources: vec![],
        plan_action: None,
        branch: None,
        max_turns: 4,
        idempotency_key: IdempotencyKey::new(format!("load-{index}")).unwrap(),
    }
}
fn snapshot(index: usize, created: String) -> GetRunResponse {
    GetRunResponse {
        run: Run {
            plugin_skill_ids: vec![],
            run_id: format!("run-{index}"),
            session_id: format!("session-{index}"),
            title: format!("Load fixture {index}"),
            role: "primary".into(),
            mode: RunMode::Execute,
            status: RunStatus::Running,
            created_at: created.clone(),
            updated_at: created,
            started_at: None,
            finished_at: None,
            last_sequence: 0,
            pending_interaction_count: 0,
            terminal: None,
            etag: "v1".into(),
            archived: false,
        },
        pending_interactions: vec![],
    }
}
#[derive(Clone)]
struct Placement {
    node: CloudNode,
    task_id: String,
    thread_id: String,
    repo: CloudRepository,
    index: usize,
}

async fn placement(
    repo: CloudRepository,
    store: Arc<CloudPostgresStore>,
    caller: CloudCaller,
    index: usize,
) -> Result<Placement, &'static str> {
    let node = repo
        .register_node(
            &caller,
            CloudNode {
                node_id: format!("node-{index}"),
                project_id: PROJECT.into(),
                instance_id: format!("instance-{index}"),
                label: format!("Load runtime {index}"),
                certificate_sha256: format!("{index:064x}"),
                roles: BTreeSet::from(["primary".into()]),
                revoked: false,
                host_id: None,
                workspace_id: None,
                workspace_label: None,
                runtime_ready: true,
                policy: None,
                policy_observed_at: None,
                revision: 0,
            },
        )
        .await
        .map_err(|_| "node enrollment failed")?;
    let (thread, task) = repo
        .create_thread(
            &caller,
            &node.node_id,
            Some(format!("Load thread {index}")),
            request(index),
        )
        .await
        .map_err(|_| "thread creation failed")?;
    let lease = store
        .claim_lease(PROJECT, &node.node_id, "load-replica", now()?, 120)
        .await
        .map_err(|_| "lease claim failed")?;
    let bound = repo.with_lease(lease);
    bound
        .record_receipt(
            &node,
            &task.task_id,
            &task.task_id,
            CloudReply::Run {
                run: Box::new(snapshot(index, timestamp()?)),
            },
        )
        .await
        .map_err(|_| "run allocation failed")?;
    Ok(Placement {
        node,
        task_id: task.task_id,
        thread_id: thread.thread_id,
        repo: bound,
        index,
    })
}

#[derive(Default)]
struct WriterResult {
    latencies: Vec<f64>,
    lag: Vec<f64>,
    committed: usize,
    errors: usize,
    final_update: Option<RunUpdate>,
}
fn update(placement: &Placement, sequence: u64, payload: usize) -> Result<RunUpdate, &'static str> {
    let created_at = timestamp()?;
    let kind = if sequence.is_multiple_of(20) {
        RunUpdateKind::Message(SessionMessage {
            session_id: format!("session-{}", placement.index),
            run_id: format!("run-{}", placement.index),
            sequence,
            role: MessageRole::Assistant,
            content: vec![MessageContentPart::Text("m".repeat(payload * 4))],
            created_at: created_at.clone(),
        })
    } else if sequence % 20 == 10 {
        RunUpdateKind::State(RunStatus::Running)
    } else {
        RunUpdateKind::OutputDelta("x".repeat(payload))
    };
    Ok(RunUpdate {
        run_id: format!("run-{}", placement.index),
        sequence,
        created_at,
        update: kind,
    })
}
async fn writer(
    placement: Placement,
    options: Options,
    rounds: usize,
    start: tokio::time::Instant,
) -> WriterResult {
    let mut result = WriterResult::default();
    for round in 0..rounds {
        let ordinal = round * options.streams + placement.index;
        let deadline = start + Duration::from_secs_f64(ordinal as f64 / options.rate as f64);
        tokio::time::sleep_until(deadline).await;
        result.lag.push(
            tokio::time::Instant::now()
                .saturating_duration_since(deadline)
                .as_secs_f64()
                * 1000.0,
        );
        let Ok(update) = update(&placement, (round + 1) as u64, options.payload) else {
            result.errors += 1;
            break;
        };
        let began = Instant::now();
        match placement
            .repo
            .record_update(&placement.node, &placement.task_id, update.clone())
            .await
        {
            Ok(_) => {
                result.committed += 1;
                result.final_update = Some(update);
            }
            Err(_) => {
                result.errors += 1;
                break;
            }
        }
        result
            .latencies
            .push(began.elapsed().as_secs_f64() * 1000.0);
    }
    result
}
#[derive(Default)]
struct ReaderResult {
    latencies: Vec<f64>,
    queries: usize,
    errors: usize,
}
async fn reader(
    repo: CloudRepository,
    caller: CloudCaller,
    placement: Placement,
    start: tokio::time::Instant,
    duration: Duration,
) -> ReaderResult {
    let mut result = ReaderResult::default();
    let end = start + duration;
    while tokio::time::Instant::now() < end {
        let began = Instant::now();
        let read = if result.queries.is_multiple_of(2) {
            repo.list_threads(&caller, None, None, Some(false), None, 100)
                .await
                .map(|_| ())
        } else {
            repo.thread_detail(&caller, &placement.thread_id, None, None)
                .await
                .map(|_| ())
        };
        result
            .latencies
            .push(began.elapsed().as_secs_f64() * 1000.0);
        result.queries += 1;
        if read.is_err() {
            result.errors += 1;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    result
}
#[derive(Serialize)]
struct Distribution {
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}
fn distribution(mut values: Vec<f64>) -> Distribution {
    values.sort_by(f64::total_cmp);
    let at = |p: f64| {
        if values.is_empty() {
            0.0
        } else {
            values[((values.len() - 1) as f64 * p).round() as usize]
        }
    };
    Distribution {
        samples: values.len(),
        p50_ms: at(0.5),
        p95_ms: at(0.95),
        max_ms: at(1.0),
    }
}

pub async fn run() -> Result<(), &'static str> {
    let options = options()?;
    let rounds = (options.rate * options.seconds).div_ceil(options.streams);
    let offered = rounds * options.streams;
    let config = CloudDatabaseConfig {
        connection_variable: "COLOSSUS_CLOUD_TEST_DATABASE_URL".into(),
        schema: format!("cloud_load_{}", uuid::Uuid::now_v7().simple()),
        tls: CloudDatabaseTls::Disabled,
        max_connections: options.pool,
        connection_timeout_ms: 5000,
        statement_timeout_ms: 15000,
    };
    let store = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .map_err(|_| "database fixture open failed")?,
    );
    store
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                key: EntityKey {
                    kind: EntityKind::Project,
                    project_id: PROJECT.into(),
                    parent_id: None,
                    id: PROJECT.into(),
                },
                expected_revision: 0,
                value: colossus_cloud::CloudProject {
                    id: PROJECT.into(),
                    name: "Load project".into(),
                    description: String::new(),
                    parent_project_id: None,
                    archived: false,
                    revision: 1,
                    created_at: String::new(),
                    updated_at: String::new(),
                }
                .into(),
                actor: "load-fixture".into(),
                operation: "cloud.load.fixture-created.v1".into(),
            }],
            ..Default::default()
        })
        .await
        .map_err(|_| "project bootstrap failed")?;
    let repo = CloudRepository::new(store.clone()).map_err(|_| "repository setup failed")?;
    let caller = CloudCaller::new(
        "load-operator".into(),
        PROJECT.into(),
        BTreeSet::from([
            CloudPermission::Administer,
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Control,
        ]),
    )
    .map_err(|_| "fixture authority failed")?;
    let placements: Vec<_> = stream::iter(0..options.streams)
        .map(|index| placement(repo.clone(), store.clone(), caller.clone(), index))
        .buffer_unordered(options.pool as usize)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()?;
    store.enable_transaction_profiling();
    let before = store.pool_statistics();
    let start = tokio::time::Instant::now() + Duration::from_millis(100);
    let began = Instant::now();
    let mut readers = tokio::task::JoinSet::new();
    for index in 0..4 {
        readers.spawn(reader(
            repo.clone(),
            caller.clone(),
            placements[index % placements.len()].clone(),
            start,
            Duration::from_secs(options.seconds as u64),
        ));
    }
    let mut writers = tokio::task::JoinSet::new();
    for placement in &placements {
        writers.spawn(writer(placement.clone(), options.clone(), rounds, start));
    }
    let mut commits = Vec::new();
    let mut lag = Vec::new();
    let mut committed = 0;
    let mut errors = 0;
    let mut final_updates = Vec::new();
    while let Some(result) = writers.join_next().await {
        let result = result.map_err(|_| "writer task failed")?;
        committed += result.committed;
        errors += result.errors;
        commits.extend(result.latencies);
        lag.extend(result.lag);
        if let Some(update) = result.final_update {
            final_updates.push(update);
        }
    }
    let elapsed = began.elapsed().as_secs_f64();
    let after = store.pool_statistics();
    let mut queries = 0;
    let mut query_errors = 0;
    let mut query_latencies = Vec::new();
    while let Some(result) = readers.join_next().await {
        let result = result.map_err(|_| "history reader task failed")?;
        queries += result.queries;
        query_errors += result.errors;
        query_latencies.extend(result.latencies);
    }
    // Reopen through a new pool and verify retained history and exact replay before continuation.
    let reopened = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .map_err(|_| "reconnect failed")?,
    );
    let resumed =
        CloudRepository::new(reopened.clone()).map_err(|_| "reconnect repository failed")?;
    let mut recovered = 0;
    let mut replayed = 0;
    let mut continued = 0;
    for placement in placements.iter().take(10) {
        let lease = reopened
            .claim_lease(
                PROJECT,
                &placement.node.node_id,
                "load-replica",
                now()?,
                120,
            )
            .await
            .map_err(|_| "reconnect lease failed")?;
        let bound = resumed.with_lease(lease);
        let task = resumed
            .get_task(&caller, &placement.task_id)
            .await
            .map_err(|_| "retained task read failed")?;
        let events = resumed
            .updates(
                &caller,
                &placement.task_id,
                task.last_sequence.saturating_sub(10),
                100,
            )
            .await
            .map_err(|_| "history backfill failed")?;
        if events.len() == task.last_sequence.min(10) as usize {
            recovered += 1;
        }
        if let Some(update) = final_updates
            .iter()
            .find(|u| u.run_id == format!("run-{}", placement.index))
        {
            bound
                .record_update(&placement.node, &placement.task_id, update.clone())
                .await
                .map_err(|_| "exact replay failed")?;
            replayed += 1;
        }
        let next = update(placement, task.last_sequence + 1, options.payload)?;
        bound
            .record_update(&placement.node, &placement.task_id, next)
            .await
            .map_err(|_| "continued stream failed")?;
        continued += 1;
    }
    let waited = after
        .waited_acquisitions
        .saturating_sub(before.waited_acquisitions);
    let wait_ms = after.total_wait_ms - before.total_wait_ms;
    let report = json!({"transaction_profile":store.transaction_profile(),"host_os":std::env::consts::OS,"host_arch":std::env::consts::ARCH,"available_parallelism":std::thread::available_parallelism().map(|v|v.get()).unwrap_or(0),"debug_assertions":cfg!(debug_assertions),"scope":"local PostgreSQL and actual cloud repository; excludes gRPC/SSE/OIDC and runtime execution","streams":options.streams,"pool_connections":options.pool,"offered_updates_per_second":options.rate,"configured_seconds":options.seconds,"output_payload_bytes":options.payload,"message_payload_bytes":options.payload*4,"mix":"90% output delta / 5% released assistant message / 5% state","offered_updates":offered,"committed_updates":committed,"failed_writer_operations":errors,"unattempted_updates":offered.saturating_sub(committed+errors),"error_rate":if offered==0{0.0}else{(offered-committed)as f64/offered as f64},"elapsed_seconds":elapsed,"actual_updates_per_second":committed as f64/elapsed,"target_met":committed as f64/elapsed>=options.rate as f64*0.95&&errors==0,"event_operation_latency":distribution(commits),"offered_queue_lag":distribution(lag),"history_queries":queries,"history_query_errors":query_errors,"history_query_latency":distribution(query_latencies),"pool_acquisitions":after.acquisitions-before.acquisitions,"pool_waited_acquisitions":waited,"pool_total_wait_ms":wait_ms,"pool_mean_waited_acquisition_ms":if waited==0{0.0}else{wait_ms/waited as f64},"pool_timeouts":after.timed_out_acquisitions-before.timed_out_acquisitions,"pool_connections_established":after.connections,"reconnect_history_streams_verified":recovered,"exact_replay_streams_verified":replayed,"continued_streams_verified":continued,"retained_schema":if options.keep{Some(config.schema.clone())}else{None}});
    let text = serde_json::to_string_pretty(&report).map_err(|_| "report serialization failed")?;
    if let Some(path) = &options.report {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
            .map_err(|_| "report file cannot be created")?;
    }
    println!("{text}");
    if !options.keep {
        store
            .remove_fixture_schema(&config.schema)
            .await
            .map_err(|_| "fixture cleanup failed")?;
    }
    Ok(())
}
