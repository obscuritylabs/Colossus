use super::*;
use colossus_api::{
    ApiError, ApiResult, ApiScope, ApplicationKind, ApplicationPrincipal, ArtifactDownload,
    ArtifactUploadReservation, CallerContext, EventSourcedArtifactApi, RequestId, scopes,
};
use colossus_testkit::InMemoryEventJournal;
use futures::StreamExt as _;
use prost::Message as _;
use sha2::{Digest as _, Sha256};
use std::{
    sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

fn caller(application: &str, scopes: &[&str]) -> CallerContext {
    CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            application,
            "credential-artifact-test",
            ApplicationKind::Enrolled,
            scopes.iter().map(|scope| ApiScope::new(*scope).unwrap()),
            ["primary".into()],
            Vec::<String>::new(),
        )
        .unwrap(),
        RequestId::new("artifact-test").unwrap(),
    )
}

fn owner() -> CallerContext {
    caller(
        "app:artifact-owner",
        &[scopes::ARTIFACTS_READ, scopes::ARTIFACTS_WRITE],
    )
}

fn request<T>(body: T, caller: CallerContext) -> Request<T> {
    let mut request = Request::new(body);
    request.extensions_mut().insert(caller);
    request
}

async fn seeded() -> (Arc<EventSourcedArtifactApi>, String, Vec<u8>) {
    let api = Arc::new(EventSourcedArtifactApi::new(Arc::new(
        InMemoryEventJournal::default(),
    )));
    let bytes: Vec<u8> = (0..ARTIFACT_CHUNK_BYTES + 37)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect();
    let reserved = api
        .create_upload(
            &owner(),
            CoreCreateArtifactUploadRequest {
                file_name: "fixture.png".into(),
                media_type: "image/png".into(),
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                purpose: CoreArtifactPurpose::RunOutput,
                idempotency_key: IdempotencyKey::new("fixture-artifact").unwrap(),
            },
        )
        .await
        .unwrap();
    let chunks = bytes
        .chunks(ARTIFACT_CHUNK_BYTES)
        .enumerate()
        .map(|(index, data)| CoreArtifactChunk {
            offset: (index * ARTIFACT_CHUNK_BYTES) as u64,
            data: data.to_vec(),
        })
        .collect();
    let artifact = api
        .upload(&owner(), &reserved.upload_id, chunks)
        .await
        .unwrap();
    (api, artifact.artifact_id, bytes)
}

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    changed: Condvar,
    entered: AtomicUsize,
}

impl Gate {
    fn block(&self) -> ApiResult<()> {
        self.entered.fetch_add(1, Ordering::AcqRel);
        let lock = self.released.lock().unwrap();
        let (_held, result) = self
            .changed
            .wait_timeout_while(lock, Duration::from_secs(5), |released| !*released)
            .unwrap();
        if result.timed_out() {
            return Err(ApiError::failed_precondition(
                colossus_api::ApiErrorReason::InternalInvariant,
                "test read gate reached its safety deadline",
            ));
        }
        Ok(())
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }

    async fn wait_entered(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.entered.load(Ordering::Acquire) < count {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the admitted storage reads must start");
    }
}

struct ReleaseOnDrop(Arc<Gate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct HeldApi {
    inner: Arc<EventSourcedArtifactApi>,
    gate: Arc<Gate>,
}

#[tonic::async_trait]
impl ArtifactApi for HeldApi {
    async fn create_upload(
        &self,
        caller: &CallerContext,
        request: CoreCreateArtifactUploadRequest,
    ) -> ApiResult<ArtifactUploadReservation> {
        self.inner.create_upload(caller, request).await
    }
    async fn upload(
        &self,
        caller: &CallerContext,
        upload_id: &str,
        chunks: Vec<CoreArtifactChunk>,
    ) -> ApiResult<CoreArtifactReference> {
        self.inner.upload(caller, upload_id, chunks).await
    }
    async fn get(&self, caller: &CallerContext, id: &str) -> ApiResult<CoreArtifactReference> {
        self.gate.block()?;
        self.inner.get(caller, id).await
    }
    async fn download(
        &self,
        caller: &CallerContext,
        id: &str,
        offset: u64,
    ) -> ApiResult<ArtifactDownload> {
        self.gate.block()?;
        self.inner.download(caller, id, offset).await
    }
}

async fn read(service: ArtifactServiceAdapter, id: String, download: bool) -> Result<(), Status> {
    if download {
        service
            .download_artifact(request(
                DownloadArtifactRequest {
                    artifact_id: id,
                    offset: 0,
                },
                owner(),
            ))
            .await
            .map(|_| ())
    } else {
        service
            .get_artifact(request(GetArtifactRequest { artifact_id: id }, owner()))
            .await
            .map(|_| ())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn held_get_and_download_leave_the_reactor_responsive() {
    for download in [false, true] {
        let (inner, id, _) = seeded().await;
        let gate = Arc::new(Gate::default());
        let _release = ReleaseOnDrop(gate.clone());
        let service = ArtifactServiceAdapter::new(Arc::new(HeldApi {
            inner,
            gate: gate.clone(),
        }));
        let (pulse, receive) = mpsc::sync_channel(1);
        let watchdog_gate = gate.clone();
        // A separate finite watchdog makes the old inline blocking path fail
        // without deadlocking the current-thread executor or test shutdown.
        let watchdog = std::thread::spawn(move || {
            let responsive = receive.recv_timeout(Duration::from_secs(2)).is_ok();
            watchdog_gate.release();
            responsive
        });
        let reading = tokio::spawn(read(service, id, download));
        gate.wait_entered(1).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = pulse.send(());
        reading.await.unwrap().unwrap();
        assert!(
            watchdog.join().unwrap(),
            "a storage read blocked the async reactor"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn shared_read_capacity_stays_charged_after_caller_cancellation() {
    let (inner, id, _) = seeded().await;
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let service = ArtifactServiceAdapter::new(Arc::new(HeldApi {
        inner,
        gate: gate.clone(),
    }));
    let mut reading = Vec::new();
    for index in 0..4 {
        reading.push(tokio::spawn(read(
            service.clone(),
            id.clone(),
            index % 2 == 0,
        )));
    }
    gate.wait_entered(4).await;
    for task in reading {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    let denied = service
        .get_artifact(request(
            GetArtifactRequest {
                artifact_id: id.clone(),
            },
            caller("app:no-scope", &[]),
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
    let full = read(service.clone(), id.clone(), false).await.unwrap_err();
    assert_eq!(full.code(), tonic::Code::ResourceExhausted);
    let status = colossus_api_proto::google_rpc::Status::decode(full.details()).unwrap();
    let detail = colossus_api_proto::v1alpha1::ColossusErrorDetail::decode(
        status.details[0].value.as_slice(),
    )
    .unwrap();
    assert_eq!(detail.reason, "capacity_exceeded");
    assert_eq!(detail.request_id, owner().request_id().as_str());
    assert_eq!(
        detail.outcome_certainty,
        colossus_api_proto::v1alpha1::OutcomeCertainty::Known as i32
    );
    gate.release();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match read(service.clone(), id.clone(), false).await {
                Ok(()) => break,
                Err(status) if status.code() == tonic::Code::ResourceExhausted => {
                    tokio::time::sleep(Duration::from_millis(1)).await
                }
                Err(status) => panic!("read failed after storage release: {status}"),
            }
        }
    })
    .await
    .expect("finished blocking reads must release their admission");
}

#[tokio::test]
async fn offloaded_reads_preserve_metadata_chunks_offsets_and_authority_errors() {
    let (api, id, bytes) = seeded().await;
    let service = ArtifactServiceAdapter::new(api);
    let artifact = service
        .get_artifact(request(
            GetArtifactRequest {
                artifact_id: id.clone(),
            },
            owner(),
        ))
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    assert_eq!(artifact.sha256, format!("{:x}", Sha256::digest(&bytes)));
    let chunks = service
        .download_artifact(request(
            DownloadArtifactRequest {
                artifact_id: id.clone(),
                offset: 13,
            },
            owner(),
        ))
        .await
        .unwrap()
        .into_inner()
        .collect::<Vec<_>>()
        .await;
    let chunks = chunks.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].offset, 13);
    assert_eq!(chunks[1].offset, 13 + ARTIFACT_CHUNK_BYTES as u64);
    assert_eq!(
        chunks
            .into_iter()
            .flat_map(|chunk| chunk.data)
            .collect::<Vec<_>>(),
        bytes[13..]
    );
    let foreign = service
        .get_artifact(request(
            GetArtifactRequest {
                artifact_id: id.clone(),
            },
            caller("app:foreign", &[scopes::ARTIFACTS_READ]),
        ))
        .await
        .unwrap_err();
    assert_eq!(foreign.code(), tonic::Code::NotFound);
    let invalid = service
        .get_artifact(request(
            GetArtifactRequest {
                artifact_id: "../private.png".into(),
            },
            owner(),
        ))
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), tonic::Code::InvalidArgument);
    let offset = service
        .download_artifact(request(
            DownloadArtifactRequest {
                artifact_id: id,
                offset: bytes.len() as u64 + 1,
            },
            owner(),
        ))
        .await;
    assert_eq!(
        offset.err().expect("invalid offset").code(),
        tonic::Code::InvalidArgument
    );
}
