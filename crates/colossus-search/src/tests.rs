use super::*;
use colossus_contracts::{
    Actor, ActorType, DecisionOutcome, EffectPhase, ExecutionContext, PolicyDecision,
    SandboxBoundaryMode,
};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectGateway, GatewayError, SafetyKernel, SandboxBoundaryGate,
    effect_request,
};
use colossus_ports::{EventJournal, PolicyDecisionPoint};
use colossus_testkit::InMemoryEventJournal;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    time::{Duration as TokioDuration, timeout},
};

struct CountingCredentialResolver {
    calls: AtomicUsize,
    secret: String,
}

#[test]
fn ambient_search_profiles_allow_non_loopback_http_only_explicitly() {
    assert!(
        SearchProfile::new(
            "private",
            SearchKind::Searxng,
            "http://169.254.169.254/search",
            None,
            None,
            "test",
            1_000,
        )
        .is_err()
    );
    SearchProfile::new_with_resource_authority(
        "private",
        SearchKind::Searxng,
        "http://169.254.169.254/search",
        None,
        None,
        "test",
        1_000,
        ResourceAuthority::Ambient,
    )
    .expect("ambient metadata search endpoint");
}

impl CredentialResolver for CountingCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, SearchAdapterError> {
        assert!(reference.starts_with("env:"));
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok(self.secret.clone())
    }
}

struct SearchPostDenyPolicy(BuiltInPolicy);

#[async_trait]
impl PolicyDecisionPoint for SearchPostDenyPolicy {
    async fn decide(
        &self,
        request: &EffectRequest,
    ) -> Result<PolicyDecision, colossus_ports::PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.phase == EffectPhase::PostEffect {
            decision.outcome = DecisionOutcome::Deny;
            decision.reason = "search content denied after quarantine".into();
        }
        Ok(decision)
    }

    async fn doctor(&self) -> Result<Value, colossus_ports::PolicyError> {
        self.0.doctor().await
    }
}

fn search_request(profile: &SearchProfile, query: &str, limit: usize) -> EffectRequest {
    let mut request = effect_request(
        Actor {
            actor_type: ActorType::User,
            id: "search-test".into(),
        },
        "web.search",
        profile.endpoint(),
        serde_json::to_value(SearchEffectInput {
            profile: profile.name().into(),
            request: SearchRequest {
                query: query.into(),
                limit,
            },
        })
        .expect("request JSON"),
    );
    request.capabilities = vec!["web.search".into()];
    request.context = ExecutionContext::default();
    request.credential_references = profile.credential_reference().into_iter().collect();
    request
}

async fn one_response_server(
    status: u16,
    body: Value,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        let mut scratch = [0_u8; 2048];
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let read = stream.read(&mut scratch).await.expect("read");
            assert_ne!(read, 0);
            request.extend_from_slice(&scratch[..read]);
        }
        let bytes = serde_json::to_vec(&body).expect("response JSON");
        let reason = if status == 200 { "OK" } else { "Found" };
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            bytes.len()
        );
        stream.write_all(head.as_bytes()).await.expect("headers");
        stream.write_all(&bytes).await.expect("body");
        String::from_utf8_lossy(&request).into_owned()
    });
    (format!("http://{address}/search"), task)
}

fn allow_gateway(origin: &str) -> EffectGateway {
    EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("web.search", DecisionOutcome::Allow)
                .with_network_destination(origin)
                .with_post_effect(true),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["web.search".into()]),
        [41_u8; 32],
    )
}

#[test]
fn profiles_routes_and_request_bounds_are_strict() {
    assert!(
        SearchProfile::new(
            "remote",
            SearchKind::Searxng,
            "http://example.com/search",
            None,
            Some("X-Key".into()),
            "test",
            1000,
        )
        .is_err()
    );
    assert!(
        SearchProfile::new(
            "paid",
            SearchKind::SerpApi,
            "https://serpapi.com/search.json",
            None,
            None,
            "test",
            1000,
        )
        .is_err()
    );
    assert!(
        validate_request(&SearchRequest {
            query: "x".repeat(4097),
            limit: 10
        })
        .is_err()
    );
    assert!(
        validate_request(&SearchRequest {
            query: "ok".into(),
            limit: 21
        })
        .is_err()
    );
    let registry = SearchRegistry::new(
        Vec::new(),
        BTreeMap::from([("agent".into(), "missing".into())]),
    );
    assert!(registry.is_err());
}

#[test]
fn normalization_drops_unsafe_urls_and_bounds_provider_output() {
    let response = normalize_response(
            SearchKind::Searxng,
            &SearchRequest { query: "q".into(), limit: 2 },
            &json!({"results": [
                {"title": "bad", "url": "file:///etc/passwd", "content": "ignored"},
                {"title": "x".repeat(5000), "url": "https://example.com/a", "content": "s", "engine": "local"},
                {"title": "second", "url": "http://example.org/b", "content": "two"},
                {"title": "third", "url": "https://example.net/c", "content": "three"}
            ]}),
        )
        .expect("normalized");
    assert_eq!(response.count, 2);
    assert_eq!(response.results[0].rank, 1);
    assert_eq!(response.results[0].title.chars().count(), MAX_TITLE_CHARS);
    assert_eq!(response.results[0].source.as_deref(), Some("local"));
    assert!(
        normalize_response(
            SearchKind::Searxng,
            &SearchRequest {
                query: "q".into(),
                limit: 1
            },
            &json!({})
        )
        .is_err()
    );
}

#[tokio::test]
async fn searxng_request_is_normalized_through_quarantine() {
    let (endpoint, server) = one_response_server(200, json!({
            "results": [{"title": "Result", "url": "https://example.com/a", "content": "Snippet", "engine": "unit"}],
            "untrusted_metadata": {"ignored": true}
        })).await;
    let profile = SearchProfile::new(
        "local",
        SearchKind::Searxng,
        endpoint,
        None,
        Some("X-Key".into()),
        "unit-test",
        5000,
    )
    .expect("profile");
    let origin = profile.network_origin().expect("origin");
    let released = allow_gateway(&origin)
        .execute(
            search_request(&profile, "rust security", 10),
            &SearchExecutor::new(profile),
        )
        .await
        .expect("search");
    let response: SearchResponse =
        serde_json::from_slice(&released.bytes).expect("normalized response");
    assert_eq!(response.results[0].snippet, "Snippet");
    assert!(!String::from_utf8_lossy(&released.bytes).contains("untrusted_metadata"));
    let request = server.await.expect("server");
    assert!(request.starts_with("GET /search?"));
    assert!(request.contains("q=rust+security"));
    assert!(request.contains("format=json"));
}

#[tokio::test]
async fn serpapi_injects_and_redacts_credentials_after_permit() {
    let secret = "unit-serp-secret";
    let (endpoint, server) = one_response_server(200, json!({
            "organic_results": [{"title": secret, "link": "https://example.com/a", "snippet": "safe", "position": 99}],
            "api_key": secret
        })).await;
    let profile = SearchProfile::new(
        "paid",
        SearchKind::SerpApi,
        endpoint,
        Some("env:SERPAPI_API_KEY".into()),
        None,
        "unit-test",
        5000,
    )
    .expect("profile");
    let origin = profile.network_origin().expect("origin");
    let credentials = Arc::new(CountingCredentialResolver {
        calls: AtomicUsize::new(0),
        secret: secret.into(),
    });
    let executor = SearchExecutor::with_credentials(
        profile.clone(),
        Arc::clone(&credentials) as Arc<dyn CredentialResolver>,
    );
    let released = allow_gateway(&origin)
        .execute(search_request(&profile, "colossus", 3), &executor)
        .await
        .expect("search");
    let response: SearchResponse =
        serde_json::from_slice(&released.bytes).expect("normalized response");
    assert_eq!(response.results[0].title, "[REDACTED]");
    assert_eq!(credentials.calls.load(Ordering::Acquire), 1);
    let request = server.await.expect("server");
    assert!(request.contains("engine=google"));
    assert!(request.contains("num=3"));
    assert!(request.contains("api_key=unit-serp-secret"));
    assert!(!String::from_utf8_lossy(&released.bytes).contains(secret));
}

#[tokio::test]
async fn denial_opens_no_socket_and_resolves_no_credential() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let endpoint = format!("http://{}/search", listener.local_addr().expect("address"));
    let profile = SearchProfile::new(
        "paid",
        SearchKind::SerpApi,
        endpoint,
        Some("env:SERPAPI_API_KEY".into()),
        None,
        "unit-test",
        5000,
    )
    .expect("profile");
    let credentials = Arc::new(CountingCredentialResolver {
        calls: AtomicUsize::new(0),
        secret: "never".into(),
    });
    let executor = SearchExecutor::with_credentials(
        profile.clone(),
        Arc::clone(&credentials) as Arc<dyn CredentialResolver>,
    );
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(BuiltInPolicy::offline_default()),
        Arc::new(DenyApproval),
        SafetyKernel::new(["web.search".into()]),
        [42_u8; 32],
    );
    assert!(matches!(
        gateway
            .execute(search_request(&profile, "q", 1), &executor)
            .await,
        Err(GatewayError::Denied(_) | GatewayError::Approval(_))
    ));
    assert_eq!(credentials.calls.load(Ordering::Acquire), 0);
    assert!(
        timeout(TokioDuration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn remote_plaintext_search_requires_ambient_authority_in_the_permit() {
    let profile = SearchProfile::new_with_resource_authority(
        "remote-http",
        SearchKind::SerpApi,
        "http://192.0.2.1:9/search",
        Some("env:SERPAPI_API_KEY".into()),
        None,
        "unit-test",
        25,
        ResourceAuthority::Ambient,
    )
    .expect("potential ambient profile");
    let origin = profile.network_origin().expect("origin");
    let credentials = Arc::new(CountingCredentialResolver {
        calls: AtomicUsize::new(0),
        secret: "unit-secret".into(),
    });
    let executor = SearchExecutor::with_credentials(
        profile.clone(),
        Arc::clone(&credentials) as Arc<dyn CredentialResolver>,
    );

    let declared = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("web.search", DecisionOutcome::Allow)
                .with_network_destination(&origin)
                .with_post_effect(true),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["web.search".into()]),
        [63_u8; 32],
    );
    let error = declared
        .execute(search_request(&profile, "q", 1), &executor)
        .await
        .expect_err("declared exact origin must not authorize remote plaintext HTTP");
    assert!(
        error
            .to_string()
            .contains("requires ambient resource authority")
    );
    assert_eq!(credentials.calls.load(Ordering::Acquire), 0);

    let ambient = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("web.search", DecisionOutcome::Allow)
                .with_sandbox("danger_full_access", "test", false)
                .with_resource_authority(ResourceAuthority::Ambient)
                .with_limits(25, 1024 * 1024, 1, 64 * 1024 * 1024, 1)
                .with_post_effect(true),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["web.search".into()]).with_sandbox_boundary_gate(Arc::new(
            SandboxBoundaryGate::new(Some(SandboxBoundaryMode::DangerFullAccess), true),
        )),
        [64_u8; 32],
    );
    let result = ambient
        .execute(search_request(&profile, "q", 1), &executor)
        .await;
    assert!(result.as_ref().err().is_none_or(|error| {
        !error
            .to_string()
            .contains("requires ambient resource authority")
    }));
    assert_eq!(credentials.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn post_effect_denial_releases_no_provider_content() {
    let private = "private-search-snippet";
    let (endpoint, server) = one_response_server(
        200,
        json!({
            "results": [{"title": "Result", "url": "https://example.com/a", "content": private}]
        }),
    )
    .await;
    let profile = SearchProfile::new(
        "local",
        SearchKind::Searxng,
        endpoint,
        None,
        Some("X-Key".into()),
        "unit-test",
        5000,
    )
    .expect("profile");
    let origin = profile.network_origin().expect("origin");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("web.search", DecisionOutcome::Allow)
        .with_network_destination(origin)
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(SearchPostDenyPolicy(policy)),
        Arc::new(DenyApproval),
        SafetyKernel::new(["web.search".into()]),
        [43_u8; 32],
    );
    let error = gateway
        .execute(
            search_request(&profile, "q", 1),
            &SearchExecutor::new(profile),
        )
        .await
        .expect_err("post denial");
    assert!(matches!(error, GatewayError::Denied(_)));
    assert!(!error.to_string().contains(private));
    server.await.expect("server");
    let events = journal.read_global(1, 50).expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.release_denied.v1")
    );
    assert!(
        !serde_json::to_string(&events)
            .expect("events JSON")
            .contains(private)
    );
}

#[tokio::test]
async fn redirects_fail_without_following_and_transport_is_outcome_unknown() {
    let (endpoint, redirect_server) = one_response_server(302, json!({})).await;
    let profile = SearchProfile::new(
        "local",
        SearchKind::Searxng,
        endpoint,
        None,
        Some("X-Key".into()),
        "unit-test",
        5000,
    )
    .expect("profile");
    let origin = profile.network_origin().expect("origin");
    let error = allow_gateway(&origin)
        .execute(
            search_request(&profile, "q", 1),
            &SearchExecutor::new(profile),
        )
        .await
        .expect_err("redirect");
    assert!(matches!(error, GatewayError::Execution(_)));
    redirect_server.await.expect("server");

    let profile = SearchProfile::new(
        "local",
        SearchKind::Searxng,
        "http://127.0.0.1:9/search",
        None,
        Some("X-Key".into()),
        "unit-test",
        100,
    )
    .expect("profile");
    let origin = profile.network_origin().expect("origin");
    let error = allow_gateway(&origin)
        .execute(
            search_request(&profile, "q", 1),
            &SearchExecutor::new(profile),
        )
        .await
        .expect_err("transport");
    assert!(matches!(error, GatewayError::OutcomeUnknown(_)));
}
