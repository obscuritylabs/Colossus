use super::*;
use std::collections::BTreeSet;

const PARENTS: [&str; 2] = ["ALPHA_PARENT", "BETA_PARENT"];
const CHILDREN: [&str; 2] = ["Alpha child", "Beta child"];

fn send_delegation(stream: &mut TcpStream, parent: usize) {
    let tool_call = json!({
        "id": "fixture-delegation",
        "choices": [{
            "index": 0,
            "delta": { "tool_calls": [{
                "index": 0,
                "id": format!("call-delegate-{parent}"),
                "type": "function",
                "function": {
                    "name": "agent_delegate",
                    "arguments": json!({"task": CHILDREN[parent]}).to_string()
                }
            }] },
            "finish_reason": "tool_calls"
        }]
    });
    let body = format!("data: {tool_call}\n\ndata: [DONE]\n\n");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    stream.flush().unwrap();
}

fn delegated_provider() -> (String, mpsc::Sender<()>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (release, released) = mpsc::channel();
    let task = thread::spawn(move || {
        let mut seen = BTreeSet::new();
        let mut running = Vec::new();
        for _ in 0..6 {
            let (mut stream, request) = accept_request(&listener);
            let messages = request["messages"].as_array().expect("provider messages");
            let text = request["messages"].to_string();
            let child = text.contains("durable Colossus child agent");
            let parent = (0..PARENTS.len())
                .find(|&index| {
                    text.contains(if child {
                        CHILDREN[index]
                    } else {
                        PARENTS[index]
                    })
                })
                .expect("provider request belongs to a fixture parent or child");
            let continuation = messages.iter().any(|message| message["role"] == "tool");
            assert!(seen.insert((parent, child, continuation)), "{text}");
            if !child && !continuation {
                assert!(
                    request["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|tool| { tool["function"]["name"] == "agent_delegate" })
                );
                send_delegation(&mut stream, parent);
                continue;
            }
            let (start, end) = if child {
                (
                    format!("PRIVATE_CHILD_{parent}"),
                    format!("CHILD_RESULT_{parent}"),
                )
            } else {
                (
                    format!("{}_RUNNING", PARENTS[parent]),
                    format!("{}_FINISHED", PARENTS[parent]),
                )
            };
            let tail = start_stream(&mut stream, &start, &end);
            running.push((stream, tail));
        }
        assert_eq!(running.len(), 4);
        released
            .recv_timeout(PROVIDER_WAIT)
            .expect("roster inspected");
        for (mut stream, tail) in running {
            stream.write_all(tail.as_bytes()).unwrap();
            stream.flush().unwrap();
        }
    });
    (origin, release, task)
}

fn open_agents(terminal: &Terminal) {
    center(terminal);
    terminal.send(b"\t\t\t");
    terminal.wait("Agents · 2 sessions");
}

fn expand_both(terminal: &Terminal) {
    terminal.send(b"\x1b[H ");
    terminal.wait(CHILDREN[0]);
    terminal.send(b"\x1b[F ");
    terminal.wait(CHILDREN[1]);
}

fn grouped(rows: &[String], status: &str) -> bool {
    rows.windows(4).any(|group| {
        group[0].contains(PARENTS[0])
            && group[1].contains(CHILDREN[0])
            && group[2].contains(PARENTS[1])
            && group[3].contains(CHILDREN[1])
            && [1, 3]
                .into_iter()
                .all(|index| group[index].contains(status) && group[index].contains("just now"))
    })
}

#[test]
fn embedded_tui_roster_groups_subagents_from_retained_sessions() {
    exercise_roster(false);
}

#[test]
fn authenticated_worker_tui_roster_groups_subagents_from_retained_sessions() {
    exercise_roster(true);
}

fn exercise_roster(worker_host: bool) {
    let directory = process_support::tempdir().expect("isolated fixture directory");
    let workspace = directory.path().canonicalize().unwrap();
    let home = process_support::isolated_user_home(&workspace);
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let (origin, release, provider) = delegated_provider();
    let mut config = provider_config(&workspace, &origin);
    config
        .models
        .roles
        .insert("subagent_default".into(), "fixture".into());
    config.subagents.max_concurrent = 2;
    config.access = serde_json::from_value(json!({
        "profile": "pinned",
        "tools": { "include": ["agent.delegate"] },
        "actions": { "allow": [
            "provider.openai.chat", "presentation.history.append", "context.show", "plugin.list",
            "subagent.create", "subagent.read", "subagent.list", "subagent.start",
            "subagent.complete", "subagent.fail", "subagent.cancel", "subagent.interrupt"
        ]}
    }))
    .unwrap();
    fs::write(
        workspace.join("config.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
    let (_worker, terminal) = start_tui(binary, &workspace, &home, worker_host);
    submit(&terminal, PARENTS[0]);
    terminal.wait("ALPHA_PARENT_RUNNING");
    terminal.send(b"\x1b[200~keep alpha draft\x1b[201~");
    terminal.wait_for_composer("Alpha unsent draft", |draft| draft == "keep alpha draft");
    center(&terminal);
    terminal.send(b"n");
    terminal.wait_for_composer("new session composer", |draft| {
        draft.contains("Implement {feature}")
    });
    submit(&terminal, PARENTS[1]);
    terminal.wait("BETA_PARENT_RUNNING");
    terminal.send(b"\x1b[200~keep beta draft\x1b[201~");
    terminal.wait_for_composer("Beta unsent draft", |draft| draft == "keep beta draft");

    terminal.resize(16, 54);
    open_agents(&terminal);
    expand_both(&terminal);
    terminal.wait_until_visual_rows(
        "both running children under their own parent",
        |screen, rows| grouped(rows, "working") && !screen.contains("PRIVATE_CHILD"),
    );
    terminal.send(b"\x1b[H\x1b[B\r");
    terminal.wait_for_composer("open a child's parent with its draft", |draft| {
        draft == "keep alpha draft"
    });
    terminal.wait("ALPHA_PARENT_RUNNING");

    open_agents(&terminal);
    expand_both(&terminal);
    release.send(()).unwrap();
    terminal.wait_until_visual_rows("both child jobs finish in place", |screen, rows| {
        grouped(rows, "done") && !screen.contains("CHILD_RESULT")
    });
    terminal.send(b"\x1b[F\r");
    terminal.wait_for_composer("Beta retained draft", |draft| draft == "keep beta draft");
    terminal.wait("BETA_PARENT_FINISHED");
    open_agents(&terminal);
    terminal.send(b"\x1b[H\r");
    terminal.wait_for_composer("Alpha retained draft", |draft| draft == "keep alpha draft");
    terminal.wait("ALPHA_PARENT_FINISHED");
    provider.join().expect("all delegated provider requests");
}
