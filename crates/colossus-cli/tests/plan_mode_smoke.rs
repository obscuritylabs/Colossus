//! Cross-process Plan Mode, single-use execution, and Goal Mode handoff acceptance.

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    thread,
    time::Duration,
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "8989898989898989898989898989898989898989898989898989898989898989";
const SIGNING_KEY: &str = "9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a";

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let count = stream.read(&mut buffer).expect("read request");
        assert_ne!(count, 0, "incomplete provider request");
        request.extend_from_slice(&buffer[..count]);
        let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .unwrap_or_default();
        if request.len() >= header_end + 4 + content_length {
            return String::from_utf8(request).expect("UTF-8 request");
        }
    }
}

fn serve(responses: Vec<&'static str>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let task = thread::spawn(move || {
        responses
            .into_iter()
            .map(|body| {
                let (mut stream, _) = listener.accept().expect("accept");
                let request = read_request(&mut stream);
                let headers = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(headers.as_bytes()).expect("headers");
                stream.write_all(body.as_bytes()).expect("body");
                stream.flush().expect("flush");
                request
            })
            .collect()
    });
    (format!("http://{address}"), task)
}

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(binary)
        .current_dir(config.parent().expect("config parent"))
        .arg("--config")
        .arg(config)
        .arg("--approval-mode")
        .arg("full-access")
        .args(arguments)
        .env("COLOSSUS_PLAN_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_PLAN_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

fn parse(output: &std::process::Output, label: &str) -> Value {
    assert!(
        output.status.success(),
        "{label}: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect(label)
}

#[test]
fn plan_mode_cannot_mutate_and_approved_plans_are_consumed_once() {
    let denied_write = r#"data: {"id":"plan-denied","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"write-1","type":"function","function":{"name":"filesystem.write","arguments":"{\"path\":\"plan-mode-escape.txt\",\"content\":\"escaped\",\"mode\":\"create\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let denied_finished = r#"data: {"id":"plan-corrected","choices":[{"index":0,"delta":{"content":"mutation-not-available"},"finish_reason":"stop"}]}

data: [DONE]

"#;
    let create_plan = r##"data: {"id":"plan-create","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"plan-1","type":"function","function":{"name":"plan.create","arguments":"{\"prompt\":\"Plan the Rust cutover\",\"content\":\"# Cutover\",\"steps\":[{\"title\":\"Implement\",\"detail\":\"Use the audited runtime\",\"requires_mutation\":true}]}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"##;
    let plan_finished = r#"data: {"id":"plan-finished","choices":[{"index":0,"delta":{"content":"draft-created"},"finish_reason":"stop"}]}

data: [DONE]

"#;
    let direct_finished = r#"data: {"id":"direct-finished","choices":[{"index":0,"delta":{"content":"direct-executed"},"finish_reason":"stop"}]}

data: [DONE]

"#;
    let goal_complete = r#"data: {"id":"goal-complete","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"goal-1","type":"function","function":{"name":"goal.update","arguments":"{\"status\":\"complete\",\"summary\":\"goal-executed\",\"blocked_reason\":\"\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let goal_finished = r#"data: {"id":"goal-finished","choices":[{"index":0,"delta":{"content":"goal-executed"},"finish_reason":"stop"}]}

data: [DONE]

"#;
    let (origin, server) = serve(vec![
        denied_write,
        denied_finished,
        create_plan,
        plan_finished,
        direct_finished,
        goal_complete,
        goal_finished,
    ]);
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let config = directory.path().join("config.json");
    fs::write(
        &config,
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": 2,
            "storage": {
                "path": directory.path().join("state.redb"),
                "keys": {
                    "kind": "environment",
                    "journal_variable": "COLOSSUS_PLAN_TEST_JOURNAL_KEY",
                    "journal_key_id": "plan-test-journal-v1",
                    "signing_variable": "COLOSSUS_PLAN_TEST_SIGNING_KEY",
                    "anchor_path": directory.path().join("anchor.json")
                }
            },
            "access": {
                "profile": "pinned",
                "tools": {
                    "include": ["filesystem.write", "plan.create", "goal.update"],
                    "exclude": []
                },
                "actions": {
                    "allow": [
                    "provider.openai.chat", "filesystem.write", "plan.create", "plan.execute",
                    "goal.create", "goal.update", "goal.iteration.record"
                    ],
                    "requireApproval": ["plan.approve_request"],
                    "deny": []
                }
            },
            "policy": {"kind": "built_in", "require_post_effect": true},
            "workflows": {"repository": workflows, "user": workflows},
            "providers": {
                "profiles": {
                    "test": {
                        "kind": "open_ai_compatible",
                        "baseUrl": format!("{origin}/v1"),
                        "credentialReference": null,
                        "timeoutMs": 5000
                    }
                }
            },
            "models": {
                "profiles": {
                    "test": {
                        "providerProfile": "test",
                        "model": "plan-test",
                        "contextWindowTokens": 32768,
                        "maxOutputTokens": 4096,
                        "capabilities": {"toolCalls": true, "streaming": true}
                    }
                },
                "roles": {"primary": "test"}
            },
            "agent": {"maxTurns": 4},
            "subagents": {"maxConcurrent": 1},
            "sandbox": {
                "backend": "native",
                "profile": "plan-test-v1",
                "allowBrokerFallback": false,
                "helperPath": null,
                "ociRuntime": null,
                "ociImage": null,
                "ociProxyImage": null,
                "filesystem": [{"root": directory.path(), "mode": "write"}],
                "executables": [],
                "environment": [],
                "networkDestinations": [origin],
                "timeoutMs": 5000,
                "maxOutputBytes": 1048576,
                "maxProcesses": 2,
                "maxMemoryBytes": 67108864,
                "maxConcurrency": 1
            }
        }))
        .expect("config JSON"),
    )
    .expect("config");

    let session = parse(
        &run(binary, &config, &["sessions", "new", "plan-test"]),
        "session JSON",
    );
    let session_id = session["id"].as_str().expect("session id");

    let denied = parse(
        &run(
            binary,
            &config,
            &[
                "run",
                "Attempt a mutation",
                "--plan",
                "--session",
                session_id,
            ],
        ),
        "plan mutation correction JSON",
    );
    assert_eq!(denied["output"], "mutation-not-available");
    assert!(!directory.path().join("plan-mode-escape.txt").exists());

    let planned = parse(
        &run(
            binary,
            &config,
            &[
                "run",
                "Plan the Rust cutover",
                "--plan",
                "--session",
                session_id,
            ],
        ),
        "plan mode JSON",
    );
    assert_eq!(planned["output"], "draft-created");
    let plans = parse(
        &run(binary, &config, &["plans", "list", "--session", session_id]),
        "plans JSON",
    );
    assert_eq!(plans.as_array().map(Vec::len), Some(1));

    let direct = parse(
        &run(
            binary,
            &config,
            &[
                "plans",
                "create",
                session_id,
                "Direct plan",
                "--step",
                "Execute",
            ],
        ),
        "direct plan JSON",
    );
    let direct_id = direct["id"].as_str().expect("direct plan id");
    parse(
        &run(binary, &config, &["plans", "approve", direct_id]),
        "direct approval JSON",
    );
    let executed = parse(
        &run(binary, &config, &["run", "--execute-plan", direct_id]),
        "direct execution JSON",
    );
    assert_eq!(executed["output"], "direct-executed");
    let replay = run(binary, &config, &["run", "--execute-plan", direct_id]);
    assert!(!replay.status.success());
    let direct = parse(
        &run(binary, &config, &["plans", "show", direct_id]),
        "executed plan JSON",
    );
    assert_eq!(direct["status"], "executed");
    assert_eq!(direct["executed_run_id"], executed["run_id"]);

    let goal_plan = parse(
        &run(
            binary,
            &config,
            &[
                "plans",
                "create",
                session_id,
                "Goal plan",
                "--step",
                "Execute",
            ],
        ),
        "goal plan JSON",
    );
    let goal_plan_id = goal_plan["id"].as_str().expect("goal plan id");
    parse(
        &run(binary, &config, &["plans", "approve", goal_plan_id]),
        "goal approval JSON",
    );
    let goal = parse(
        &run(
            binary,
            &config,
            &[
                "run",
                "--execute-plan",
                goal_plan_id,
                "--goal",
                "--goal-max-iterations",
                "1",
            ],
        ),
        "goal execution JSON",
    );
    assert_eq!(goal["goal"]["status"], "complete");
    assert_eq!(goal["goal"]["source_plan_id"], goal_plan_id);
    let goal_plan = parse(
        &run(binary, &config, &["plans", "show", goal_plan_id]),
        "goal-consumed plan JSON",
    );
    assert_eq!(goal_plan["status"], "executed");
    assert_eq!(goal_plan["executed_run_id"], goal["goal"]["id"]);

    let requests = server.join().expect("provider server");
    let denied_body = requests[0].split("\r\n\r\n").nth(1).expect("body");
    assert!(!denied_body.contains("filesystem.write"));
    assert!(denied_body.contains("plan.create"));
    let correction_body = requests[1].split("\r\n\r\n").nth(1).expect("body");
    assert!(correction_body.contains("not available in this run mode"));
    assert!(correction_body.contains("unknown_tool"));
}
