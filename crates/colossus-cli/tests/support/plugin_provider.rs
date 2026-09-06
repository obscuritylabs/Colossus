//! Actual provider-input and immutable-catalog acceptance through the authenticated worker.

use super::*;

const SELECTED: &str = "colossus/plugin-authoring";
const INSTRUCTION: &str = "# Author a Colossus Agent Plugin";
const FINAL: &str = "data: {\"id\":\"plugin-final\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"plugin acceptance complete\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
const READ: &str = "data: {\"id\":\"plugin-read\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"read-core\",\"type\":\"function\",\"function\":{\"name\":\"plugin_skill_read\",\"arguments\":\"{\\\"skill\\\":\\\"colossus/plugin-authoring\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";

fn accept(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).expect("blocking stream");
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "provider request timed out");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("provider accept: {error}"),
        }
    }
}

fn success(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON")
}

#[test]
fn worker_plugin_inputs_are_progressive_and_lifecycle_changes_affect_only_new_runs() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("workspace");
    let home = process_support::isolated_user_home(directory.path());
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let origin = format!("http://{}", listener.local_addr().expect("provider origin"));
    let config = write_doctor_config(directory.path(), &origin);
    let mut document: Value =
        serde_json::from_slice(&fs::read(&config).expect("config")).expect("configuration JSON");
    document["access"]["tools"]["include"] = json!(["plugin.skill.read"]);
    document["access"]["actions"]["allow"] = json!([
        "provider.openai.chat",
        "plugin.list",
        "plugin.skill.read",
        "plugin.disable"
    ]);
    fs::write(&config, serde_json::to_vec(&document).expect("config JSON")).expect("config");
    let invoke = || {
        let mut operation = command(binary, &config);
        operation.env("COLOSSUS_HOME", home.colossus_home());
        operation
    };
    let mut worker = ChildGuard(
        invoke()
            .arg("worker")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("worker"),
    );
    wait_for_worker(binary, &config);
    let inventory = success(
        invoke()
            .args(["plugins", "list"])
            .output()
            .expect("inventory"),
    );
    let digest = inventory[0]["digest"]
        .as_str()
        .expect("core digest")
        .to_owned();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..5 {
            let mut stream = accept(&listener);
            requests.push(request_body(&read_request(&mut stream)));
            if index == 2 {
                started_tx.send(()).expect("started");
                continue_rx
                    .recv_timeout(Duration::from_secs(20))
                    .expect("disable completed");
            }
            respond_sse(&mut stream, if index == 2 { READ } else { FINAL });
        }
        requests
    });
    success(
        invoke()
            .args(["run", "metadata only"])
            .output()
            .expect("metadata run"),
    );
    success(
        invoke()
            .args(["run", "explicit selection", "--skill", SELECTED])
            .output()
            .expect("selected run"),
    );
    let active = invoke()
        .args(["run", "read after disable", "--skill", SELECTED])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("active run");
    started_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("provider reached");
    success(
        invoke()
            .args(["plugins", "disable", "colossus"])
            .output()
            .expect("disable"),
    );
    continue_tx.send(()).expect("resume provider");
    success(active.wait_with_output().expect("old run"));
    let stale = invoke()
        .args(["run", "unavailable", "--skill", SELECTED])
        .output()
        .expect("stale selection");
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains(SELECTED));
    success(
        invoke()
            .args(["run", "after disable"])
            .output()
            .expect("new run"),
    );
    let requests = server.join().expect("provider server");
    let messages = |index: usize| requests[index]["messages"].to_string();
    assert!(messages(0).contains(SELECTED));
    assert!(!messages(0).contains(INSTRUCTION));
    assert!(messages(1).contains(INSTRUCTION));
    assert!(messages(2).contains(INSTRUCTION));
    assert!(
        requests[3]["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .any(|message| {
                message["role"] == "tool"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains(INSTRUCTION))
            }),
        "snapshot-bound skill read: {}",
        requests[3]
    );
    assert!(!messages(4).contains(SELECTED));
    assert_eq!(
        requests[0]["tools"], requests[1]["tools"],
        "selection grants no tools"
    );
    let audit = success(
        invoke()
            .args(["audit", "show", "--limit", "300"])
            .output()
            .expect("audit"),
    );
    assert!(
        audit.as_array().expect("events").iter().any(|event| {
            event["context"]["plugin_digests"]["colossus"] == digest
                && event["context"]["skill_ids"] == json!([SELECTED])
        }),
        "resolved selection evidence missing: {audit}"
    );
    success(
        invoke()
            .args(["worker", "--shutdown"])
            .output()
            .expect("shutdown"),
    );
    wait_for_exit(&mut worker.0);
}
