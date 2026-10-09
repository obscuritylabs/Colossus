use super::*;
use colossus_contracts::{DecisionOutcome, EffectPhase, PolicyDecision};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel, effect_request, system_actor,
};
use colossus_ports::{EventJournal, PolicyDecisionPoint, PolicyError};
use colossus_testkit::InMemoryEventJournal;
use reqwest::header::{HeaderMap, HeaderValue, LOCATION};

mod tls;

async fn read_headers(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> String {
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut buffer = [0_u8; 1024];
        let count = stream.read(&mut buffer).await.expect("read request");
        assert_ne!(count, 0, "incomplete headers");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 65536, "bounded request");
    }
    String::from_utf8(bytes).expect("request text")
}

struct Fixture {
    origin: String,
    task: tokio::task::JoinHandle<Vec<String>>,
}

async fn fixture(responses: impl FnOnce(&str) -> Vec<String>) -> Fixture {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let origin = format!("http://{}", listener.local_addr().expect("address"));
    let responses = responses(&origin);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .expect("accept deadline")
                .expect("accept");
            requests.push(read_headers(&mut stream).await);
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response");
        }
        requests
    });
    Fixture { origin, task }
}

fn redirect(status: u16, location: &str) -> String {
    format!(
        "HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nSet-Cookie: saml=private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn policy(origins: &[&str]) -> BuiltInPolicy {
    origins.iter().fold(
        BuiltInPolicy::offline_default().with_action("network.http", DecisionOutcome::Allow),
        |policy, origin| policy.with_network_destination(*origin),
    )
}

fn gateway(
    policy: impl PolicyDecisionPoint + 'static,
) -> (EffectGateway, Arc<InMemoryEventJournal>) {
    let journal = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        journal.clone(),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["network.http".into()]),
        [73_u8; 32],
    );
    (gateway, journal)
}

fn get(url: &str) -> EffectRequest {
    let mut request = effect_request(
        system_actor("redirect-test"),
        "network.http",
        url,
        json!({"method": "GET", "headers": {"accept": "*/*"}}),
    );
    request.capabilities = vec!["network.http".into()];
    request
}

#[tokio::test]
async fn supported_redirects_preserve_saml_queries_and_do_not_carry_cookies() {
    let server = fixture(|origin| {
        vec![
            redirect(301, "../idp?SAMLRequest=a%2Bb&RelayState=x%2Fy"),
            redirect(302, "?SAMLResponse=c%2Bd"),
            redirect(303, "/callback"),
            redirect(307, &format!("{origin}/continue")),
            redirect(
                308,
                &format!("{}/finish#section", origin.trim_start_matches("http:")),
            ),
            ok("landing page"),
        ]
    })
    .await;
    let (gateway, journal) = gateway(policy(&[&server.origin]));
    let result = gateway
        .execute(
            get(&format!("{}/app/start", server.origin)),
            &HttpExecutor::new(),
        )
        .await
        .expect("redirect chain");
    assert_eq!(result.bytes, b"landing page");
    assert_eq!(result.media_type, "text/plain");
    let requests = server.task.await.expect("server");
    let paths = [
        "/app/start",
        "/idp?SAMLRequest=a%2Bb&RelayState=x%2Fy",
        "/idp?SAMLResponse=c%2Bd",
        "/callback",
        "/continue",
        "/finish",
    ];
    for (request, path) in requests.iter().zip(paths) {
        assert!(
            request.starts_with(&format!("GET {path} HTTP/1.1\r\n")),
            "{request}"
        );
        let lower = request.to_ascii_lowercase();
        assert!(!lower.contains("\r\ncookie:"));
        assert!(!lower.contains("\r\nauthorization:"));
    }
    let events = journal.read_global(1, 100).expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.release_requested.v1")
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "effect.started.v1")
            .count(),
        1
    );
}

#[tokio::test]
async fn cross_origin_redirect_requires_the_target_grant() {
    let target = fixture(|_| vec![ok("identity provider")]).await;
    let source = fixture(|_| vec![redirect(302, &format!("{}/saml", target.origin))]).await;
    let (gateway, _) = gateway(policy(&[&source.origin, &target.origin]));
    let result = gateway
        .execute(get(&source.origin), &HttpExecutor::new())
        .await
        .expect("permitted target");
    assert_eq!(result.bytes, b"identity provider");
    source.task.await.expect("source");
    target.task.await.expect("target");
}

#[tokio::test]
async fn forbidden_redirect_targets_are_rejected_before_connecting() {
    let target = TcpListener::bind(("127.0.0.1", 0)).await.expect("target");
    let address = target.local_addr().expect("target address");
    for location in [
        format!("http://{address}/capture?SAMLResponse=private"),
        format!("http://localhost:{}/capture", address.port()),
        format!("http://user:private@{address}/capture"),
        "http://169.254.169.254/latest/meta-data".into(),
        "http://[::1]/capture".into(),
        "file:///private".into(),
    ] {
        let source = fixture(|_| vec![redirect(302, &location)]).await;
        let (gateway, journal) = gateway(policy(&[&source.origin, "*"]));
        let error = gateway
            .execute(get(&source.origin), &HttpExecutor::new())
            .await
            .expect_err("denied target");
        assert!(!error.to_string().contains("private"));
        assert!(
            tokio::time::timeout(Duration::from_millis(25), target.accept())
                .await
                .is_err()
        );
        assert!(
            !journal
                .read_global(1, 100)
                .expect("events")
                .iter()
                .any(|event| event.event_type == "effect.release_requested.v1")
        );
        source.task.await.expect("source");
    }
}

#[tokio::test]
async fn redirect_limit_counts_hops_and_zero_disables_following() {
    for limit in [0, 1] {
        let server = fixture(|_| {
            (0..=limit)
                .map(|index| redirect(302, &format!("/{}", index + 1)))
                .collect()
        })
        .await;
        let (gateway, _) = gateway(policy(&[&server.origin]));
        let executor = HttpExecutor::new()
            .with_max_redirects(limit)
            .expect("limit");
        let error = gateway
            .execute(get(&format!("{}/0", server.origin)), &executor)
            .await
            .expect_err("limit");
        assert!(
            error
                .to_string()
                .contains(&format!("maxRedirects: {limit}"))
        );
        assert_eq!(server.task.await.expect("server").len(), limit + 1);
    }
}

#[tokio::test]
async fn redirect_cycles_ignore_fragments() {
    let server = fixture(|_| vec![redirect(302, "/login"), redirect(302, "/start#section")]).await;
    let (gateway, _) = gateway(policy(&[&server.origin]));
    let error = gateway
        .execute(
            get(&format!("{}/start", server.origin)),
            &HttpExecutor::new(),
        )
        .await
        .expect_err("loop");
    assert!(error.to_string().contains("redirect loop"));
    assert_eq!(server.task.await.expect("server").len(), 2);
}

#[tokio::test]
async fn writes_and_body_bearing_gets_never_follow_redirects() {
    let target = TcpListener::bind(("127.0.0.1", 0)).await.expect("target");
    let target_origin = format!("http://{}", target.local_addr().expect("address"));
    for (method, body) in [
        ("POST", None),
        ("PUT", None),
        ("DELETE", None),
        ("GET", Some("private")),
    ] {
        let source = fixture(|_| vec![redirect(303, &target_origin)]).await;
        let (gateway, _) = gateway(policy(&[&source.origin, &target_origin]));
        let mut request = get(&source.origin);
        request.content["method"] = json!(method);
        if let Some(body) = body {
            request.content["body_base64"] = json!(BASE64.encode(body));
        }
        let error = gateway
            .execute(request, &HttpExecutor::new())
            .await
            .expect_err("no replay");
        assert!(error.to_string().contains("303"));
        assert!(
            tokio::time::timeout(Duration::from_millis(25), target.accept())
                .await
                .is_err()
        );
        assert_eq!(source.task.await.expect("source").len(), 1);
    }
}

#[test]
fn invalid_locations_and_https_downgrades_fail_without_exposing_urls() {
    let current = Url::parse("https://example.test/start").expect("URL");
    let oversized = format!("/{}", "x".repeat(8192));
    for location in [
        None,
        Some(""),
        Some("https://["),
        Some(oversized.as_str()),
        Some("http://example.test/?SAMLResponse=private"),
    ] {
        let mut headers = HeaderMap::new();
        if let Some(location) = location {
            headers.insert(LOCATION, HeaderValue::from_str(location).expect("header"));
        }
        let error = redirects::RedirectChain::new(&current, 10)
            .next(&current, &headers)
            .expect_err("invalid location");
        assert!(!error.to_string().contains("private"));
    }
    let mut headers = HeaderMap::new();
    headers.append(LOCATION, HeaderValue::from_static("/first"));
    headers.append(LOCATION, HeaderValue::from_static("/second"));
    assert!(
        redirects::RedirectChain::new(&current, 10)
            .next(&current, &headers)
            .is_err()
    );
    headers.clear();
    headers.insert(
        LOCATION,
        HeaderValue::from_bytes(&[255]).expect("opaque header"),
    );
    assert!(
        redirects::RedirectChain::new(&current, 10)
            .next(&current, &headers)
            .is_err()
    );
}

#[test]
fn adapter_rejects_a_redirect_limit_above_the_hard_ceiling() {
    assert!(
        HttpExecutor::new()
            .with_max_redirects(MAX_HTTP_REDIRECTS)
            .is_ok()
    );
    assert!(
        HttpExecutor::new()
            .with_max_redirects(MAX_HTTP_REDIRECTS + 1)
            .is_err()
    );
}

#[tokio::test]
async fn head_redirects_preserve_the_method() {
    let server = fixture(|_| vec![redirect(303, "/final"), ok("not downloaded")]).await;
    let (gateway, _) = gateway(policy(&[&server.origin]));
    let mut request = get(&server.origin);
    request.content["method"] = json!("HEAD");
    let result = gateway
        .execute(request, &HttpExecutor::new())
        .await
        .expect("HEAD chain");
    assert!(result.bytes.is_empty());
    let requests = server.task.await.expect("server");
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.starts_with("HEAD ")));
}

#[tokio::test]
async fn oversized_initial_urls_fail_before_connecting() {
    let target = TcpListener::bind(("127.0.0.1", 0)).await.expect("target");
    let origin = format!("http://{}", target.local_addr().expect("address"));
    let (gateway, _) = gateway(policy(&[&origin]));
    let request = get(&format!("{origin}/{}", "x".repeat(8192)));
    let error = gateway
        .execute(request, &HttpExecutor::new())
        .await
        .expect_err("bounded URL");
    assert!(error.to_string().contains("URL exceeds 8192 bytes"));
    assert!(
        tokio::time::timeout(Duration::from_millis(25), target.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn exact_ipv6_loopback_grants_use_the_shared_host_parser() {
    let listener = TcpListener::bind(("::1", 0))
        .await
        .expect("IPv6 loopback listener");
    let origin = format!("http://{}", listener.local_addr().expect("address"));
    let server = tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("accept deadline")
            .expect("accept");
        let _ = read_headers(&mut stream).await;
        stream
            .write_all(ok("IPv6 response").as_bytes())
            .await
            .expect("response");
    });
    let (gateway, _) = gateway(policy(&[&origin]));
    let result = gateway
        .execute(get(&origin), &HttpExecutor::new())
        .await
        .expect("IPv6 request");
    assert_eq!(result.bytes, b"IPv6 response");
    server.await.expect("server");
}

#[tokio::test]
async fn the_gateway_deadline_bounds_the_whole_redirect_chain() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let origin = format!("http://{}", listener.local_addr().expect("address"));
    let server = tokio::spawn(async move {
        for index in 0..3 {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let _ = read_headers(&mut stream).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            if stream
                .write_all(redirect(302, &format!("/{}", index + 1)).as_bytes())
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let (gateway, journal) =
        gateway(policy(&[&origin]).with_limits(250, 1024, 1, 64 * 1024 * 1024, 1));
    let result = gateway
        .execute(get(&format!("{origin}/0")), &HttpExecutor::new())
        .await;
    server.abort();
    assert!(
        result.is_err(),
        "an unfinished chain must not escape the deadline"
    );
    let events = journal.read_global(1, 100).expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.outcome_unknown.v1")
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == "effect.release_requested.v1")
    );
}

struct PostDeny(BuiltInPolicy);

#[async_trait]
impl PolicyDecisionPoint for PostDeny {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.phase == EffectPhase::PostEffect {
            decision.outcome = DecisionOutcome::Deny;
            decision.reason = "redirect body denied".into();
        }
        Ok(decision)
    }

    async fn doctor(&self) -> Result<Value, PolicyError> {
        self.0.doctor().await
    }
}

#[tokio::test]
async fn redirected_body_keeps_output_bounds_and_post_effect_policy() {
    for post_deny in [false, true] {
        let server = fixture(|_| vec![redirect(302, "/final"), ok("private landing page")]).await;
        let policy = policy(&[&server.origin]);
        let (gateway, journal) = if post_deny {
            gateway(PostDeny(policy))
        } else {
            gateway(policy.with_limits(5000, 4, 1, 64 * 1024 * 1024, 1))
        };
        let error = gateway
            .execute(get(&server.origin), &HttpExecutor::new())
            .await
            .expect_err("no release");
        assert!(!error.to_string().contains("private landing page"));
        assert!(error.to_string().contains(if post_deny {
            "redirect body denied"
        } else {
            "permitted bound"
        }));
        let events = journal.read_global(1, 100).expect("events");
        assert!(
            !serde_json::to_string(&events)
                .expect("events JSON")
                .contains("private landing page")
        );
        server.task.await.expect("server");
    }
}

#[tokio::test]
async fn redirect_transport_errors_do_not_journal_saml_query_secrets() {
    let target = TcpListener::bind(("127.0.0.1", 0)).await.expect("target");
    let target_origin = format!("http://{}", target.local_addr().expect("address"));
    drop(target);
    let source = fixture(|_| {
        vec![redirect(
            302,
            &format!("{target_origin}/?SAMLResponse=private-assertion"),
        )]
    })
    .await;
    let (gateway, journal) = gateway(policy(&[&source.origin, &target_origin]));
    let error = gateway
        .execute(get(&source.origin), &HttpExecutor::new())
        .await
        .expect_err("transport failure");
    assert!(!error.to_string().contains("private-assertion"));
    assert!(
        !serde_json::to_string(&journal.read_global(1, 100).expect("events"))
            .expect("events JSON")
            .contains("private-assertion")
    );
    source.task.await.expect("source");
}
