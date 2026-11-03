use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{EffectPhase, EffectRequest, FilesystemGrant, PolicyDecision};

const MEDIA_BYTES: usize = 16 * 1024 * 1024;
const MEDIA_ACTIONS: [&str; 2] = ["filesystem.read_run_input", "filesystem.read_image"];

fn assert_input_limit(kernel: &SafetyKernel, request: &EffectRequest, expected: usize) {
    assert!(matches!(kernel.policy_projection(request),
        Err(GatewayError::Policy(PolicyError::InputTooLarge { limit })) if limit == expected));
}

#[test]
fn complete_sixteen_mib_media_disclosure_has_an_exact_post_only_lane_and_strict_override() {
    let mut bytes = vec![7_u8; MEDIA_BYTES];
    bytes[MEDIA_BYTES - 1] = 9;
    let kernel = SafetyKernel::new([]);
    let strict = SafetyKernel::new([]).with_policy_input_limit(1024);
    let mut request = effect_request(
        system_actor("media-policy-bound"),
        MEDIA_ACTIONS[0],
        "media:fixture",
        serde_json::json!({"media_type": "application/octet-stream", "size": bytes.len(),
            "content_base64": BASE64.encode(&bytes)}),
    );
    for action in MEDIA_ACTIONS {
        request.action = action.into();
        request.phase = EffectPhase::PostEffect;
        let projected = kernel.policy_projection(&request).unwrap();
        assert_eq!(projected.content, request.content);
        assert_eq!(
            BASE64
                .decode(projected.content["content_base64"].as_str().unwrap())
                .unwrap(),
            bytes
        );
        drop(projected);
        assert_input_limit(&strict, &request, 1024);
        assert!(matches!(
            strict.prepare(&request),
            Err(GatewayError::Policy(PolicyError::InputTooLarge {
                limit: 1024
            }))
        ));
        request.phase = EffectPhase::PreEffect;
        assert_input_limit(&kernel, &request, 1024 * 1024);
    }
    request.phase = EffectPhase::PostEffect;
    for ordinary in [
        "filesystem.read",
        "filesystem.read_run_input.extra",
        "filesystem.read_image.extra",
        "provider.openai.responses",
    ] {
        request.action = ordinary.into();
        assert_input_limit(&kernel, &request, 8 * 1024 * 1024);
        request.phase = EffectPhase::PreEffect;
        assert_input_limit(&kernel, &request, 1024 * 1024);
        request.phase = EffectPhase::PostEffect;
    }
}

struct FullMediaPostDeny {
    read_root: String,
    expected_hash: String,
    post_calls: AtomicUsize,
}

#[async_trait]
impl PolicyDecisionPoint for FullMediaPostDeny {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let post = request.phase == EffectPhase::PostEffect;
        if post {
            assert!(MEDIA_ACTIONS.contains(&request.action.as_str()));
            assert_eq!(request.content["size"].as_u64(), Some(MEDIA_BYTES as u64));
            let complete = BASE64
                .decode(request.content["content_base64"].as_str().unwrap())
                .unwrap();
            assert_eq!(complete.len(), MEDIA_BYTES);
            assert_eq!(crate::sha256_hex(&complete), self.expected_hash);
            self.post_calls.fetch_add(1, Ordering::AcqRel);
        }
        let mut obligations = crate::default_obligations();
        obligations.sandbox_backend = "native".into();
        obligations.filesystem = vec![FilesystemGrant {
            root: self.read_root.clone(),
            mode: "read".into(),
        }];
        obligations.max_output_bytes = MEDIA_BYTES as u64;
        obligations.require_post_effect = true;
        Ok(PolicyDecision {
            decision_id: uuid::Uuid::now_v7().to_string(),
            policy_revision: "complete-media-post-deny".into(),
            outcome: if post {
                DecisionOutcome::Deny
            } else {
                DecisionOutcome::Allow
            },
            reason: "media fixture release decision".into(),
            obligations,
        })
    }

    async fn doctor(&self) -> Result<serde_json::Value, PolicyError> {
        Ok(serde_json::json!({"ready": true}))
    }
}

#[tokio::test]
async fn media_post_denial_sees_every_quarantined_byte_and_releases_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("media.bin");
    std::fs::write(&file, b"synthetic adapter resource").unwrap();
    let tail = "PRIVATE_MEDIA_TAIL_SENTINEL";
    let secret = "x".repeat(MEDIA_BYTES - tail.len()) + tail;
    for action in MEDIA_ACTIONS {
        let policy = Arc::new(FullMediaPostDeny {
            read_root: directory.path().display().to_string(),
            expected_hash: crate::sha256_hex(secret.as_bytes()),
            post_calls: AtomicUsize::new(0),
        });
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            policy.clone(),
            Arc::new(AllowApproval {
                approved_by: "fixture".into(),
            }),
            SafetyKernel::new([]),
            [27_u8; 32],
        );
        let executor = SecretExecutor {
            calls: AtomicUsize::new(0),
            secret: secret.clone(),
        };
        let error = gateway
            .execute(
                effect_request(
                    system_actor("media-post-deny"),
                    action,
                    file.display().to_string(),
                    serde_json::json!({}),
                ),
                &executor,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, GatewayError::Denied(_)));
        assert!(!error.to_string().contains(tail));
        assert_eq!(executor.calls.load(Ordering::Acquire), 1);
        assert_eq!(policy.post_calls.load(Ordering::Acquire), 1);
        let events = journal.read_global(1, 30).unwrap();
        let types = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert!(types.contains(&"effect.started.v1"));
        assert!(types.contains(&"effect.release_requested.v1"));
        assert!(types.contains(&"effect.release_denied.v1"));
        assert!(!types.contains(&"effect.completed.v1"));
        assert!(!types.contains(&"effect.chunk_released.v1"));
        let audit = serde_json::to_string(&events).unwrap();
        assert!(!audit.contains(tail));
        assert!(!audit.contains(&BASE64.encode(&secret.as_bytes()[..48])));
    }
}
