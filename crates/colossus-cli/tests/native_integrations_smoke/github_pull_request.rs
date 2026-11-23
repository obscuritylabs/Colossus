use super::*;
use serde_json::json;
use std::{io, time::Instant};

const HEAD: &str = "0123456789012345678901234567890123456789";
const NEW_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REPO: &str = "maintainers/tools";

pub(super) fn assert_exact_head_observation(
    binary: &Path,
    config: &Path,
    workspace: &Path,
    listener: TcpListener,
) {
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let foreign = TcpListener::bind("127.0.0.1:0").expect("foreign listener");
    foreign.set_nonblocking(true).expect("nonblocking listener");
    let foreign_address = foreign.local_addr().expect("foreign address");
    let server = thread::spawn(move || {
        let pull = |number, head, repository| {
            json!({
                "number":number,
                "state":"open",
                "draft":false,
                "html_url":format!("http://{foreign_address}/ignored"),
                "head":{"sha":head,"repo":{"full_name":repository}}
            })
            .to_string()
        };
        let first = (1..=10)
            .map(|id| {
                json!({
                    "id":id, "name":"queue github-secret", "head_sha":HEAD,
                    "status":"in_progress", "conclusion":null
                })
            })
            .collect::<Vec<_>>();
        let check_page = json!({"total_count":11,"check_runs":first})
            .to_string()
            .replace("github-secret", "github\\u002dsecret");
        let second_page = json!({"total_count":11,"check_runs":[{
            "id":11,"name":"Build","head_sha":HEAD,"status":"completed","conclusion":"failure"
        }]})
        .to_string();
        let commits = format!("/repos/{REPO}/commits/{HEAD}");
        let status = json!({"sha":HEAD,"total_count":1,"statuses":[
            {"id":100,"context":"legacy/lint","state":"success"}
        ]})
        .to_string();
        let check_link = format!("Link: <http://{foreign_address}/ignored>; rel=\"next\"\r\n");
        let pull_a = format!("/repos/{REPO}/pulls/4821");
        let pull_b = format!("/repos/{REPO}/pulls/4822");
        let checks_a = format!("{commits}/check-runs?per_page=10&page=1");
        let checks_b = format!("{commits}/check-runs?per_page=10&page=2");
        let statuses = format!("{commits}/status?per_page=10&page=1");
        let same = pull(4821, HEAD, REPO);
        let fork = pull(4822, HEAD, "contributor/tools");
        let moved = pull(4822, NEW_HEAD, "contributor/tools");
        let responses = [
            (&pull_a, "200 OK", "", same.as_str()),
            (
                &checks_a,
                "200 OK",
                check_link.as_str(),
                check_page.as_str(),
            ),
            (&checks_b, "200 OK", "", second_page.as_str()),
            (&statuses, "200 OK", "", status.as_str()),
            (&pull_a, "200 OK", "", same.as_str()),
            (&pull_b, "200 OK", "", fork.as_str()),
            (
                &checks_a,
                "404 Not Found",
                "",
                r#"{"message":"github-secret"}"#,
            ),
            (
                &statuses,
                "429 Too Many Requests",
                "Retry-After: 60\r\n",
                "{}",
            ),
            (&pull_b, "200 OK", "", moved.as_str()),
        ];
        for (path, status, headers, body) in responses {
            let end = Instant::now() + Duration::from_secs(30);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < end =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("expected GET {path}: {error}"),
                }
            };
            let request = read_request(&mut stream);
            assert!(request.starts_with(&format!("GET {path} HTTP/1.1")));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer github-secret")
            );
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
                body.len()
            ).expect("GitHub fixture response");
        }
    });
    let call = |number| {
        let arguments = json!({"owner":"maintainers","repo":"tools","number":number}).to_string();
        let output = run(
            binary,
            config,
            workspace,
            &[
                "--approval-mode",
                "full-access",
                "integrations",
                "call",
                "github.pull_request",
                &arguments,
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).expect("UTF-8");
        assert!(!text.contains("github-secret"));
        assert!(!text.contains("ignored"));
        serde_json::from_str::<Value>(&text).expect("observation JSON")
    };

    let observed = call(4821);
    let result = &observed["result"];
    assert_eq!(result["observed_head_sha"], HEAD);
    assert_eq!(result["evidence_state"], "complete");
    assert_eq!(result["has_observed_pending"], true);
    assert_eq!(result["check_runs"]["items"][0]["name"], "queue [REDACTED]");
    assert_eq!(result["check_runs"]["items"][10]["conclusion"], "failure");
    assert_eq!(
        result["commit_statuses"]["items"][0]["context"],
        "legacy/lint"
    );

    let stale = call(4822);
    let result = &stale["result"];
    assert_eq!(result["observed_head_sha"], HEAD);
    assert_eq!(result["evidence_state"], "stale");
    assert_eq!(result["freshness"], "stale");
    assert_eq!(
        result["check_runs"]["incomplete"],
        json!(["fork_or_unknown_head_repository", "not_found"])
    );
    assert_eq!(
        result["commit_statuses"]["incomplete"],
        json!(["fork_or_unknown_head_repository", "rate_limited"])
    );
    assert!(
        foreign.accept().is_err(),
        "provider-supplied URL was followed"
    );
    server.join().expect("GitHub observation server");
}
