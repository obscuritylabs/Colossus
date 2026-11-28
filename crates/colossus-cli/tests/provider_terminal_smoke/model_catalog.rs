use super::*;

const CATALOG: &str = r#"{"data":[{"id":"catalog-new","name":"Discovered Model","context_length":65536,"top_provider":{"max_completion_tokens":8192}}]}"#;
const RESPONSE: &str = r#"{"id":"picker-turn","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"DISCOVERED MODEL REPLIED"}}]}"#;
const STREAM: &str = "data: {\"id\":\"picker-turn\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"DISCOVERED MODEL REPLIED\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

fn accept_until(listener: &TcpListener, deadline: Instant) -> TcpStream {
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "provider did not receive a request"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("provider accept: {error}"),
        }
    }
}

#[test]
fn tui_model_catalog_embedded_exit_settles_a_dispatched_read() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("workspace");
    let home = process_support::isolated_user_home(directory.path());
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider");
    listener.set_nonblocking(true).expect("bounded accept");
    let config = write_doctor_config(
        directory.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    );
    let (dispatched, received) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut stream = accept_until(&listener, Instant::now() + Duration::from_secs(30));
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET /v1/models "));
        dispatched.send(()).unwrap();
        held.recv_timeout(Duration::from_secs(10))
            .expect("release catalog");
        respond_json(&mut stream, "200 OK", CATALOG);
    });
    let mut terminal = catalog_terminal(binary, &config, &home, false);
    open_picker(&terminal);
    expand_live(&terminal);
    received
        .recv_timeout(Duration::from_secs(5))
        .expect("catalog dispatched");
    terminal.send(b"\x1b");
    terminal.wait("Enter sends");
    terminal.send(b"/exit\r");
    terminal.wait_until("restored terminal", |screen| {
        !screen.contains("Enter sends")
    });
    release.send(()).unwrap();
    assert!(terminal.wait_for_exit().success());
    server.join().unwrap();

    let audit = goal_usage_process(binary, &config, &home)
        .env("COLOSSUS_PROVIDER_TERMINAL_API_KEY", "terminal-secret")
        .args(["audit", "show", "--limit", "200"])
        .output()
        .expect("audit evidence");
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
    let records: Vec<Value> = serde_json::from_slice(&audit.stdout).unwrap();
    let started = records
        .iter()
        .find(|event| {
            event["event_type"] == "effect.started.v1"
                && event["actor"]["id"] == "provider-diagnostics"
        })
        .expect("provider catalog audit start");
    assert!(records.iter().any(|event| {
        event["stream_id"] == started["stream_id"] && event["event_type"] == "effect.completed.v1"
    }));
}

fn catalog_terminal(
    binary: &Path,
    config: &Path,
    home: &process_support::IsolatedUserHome,
    with_worker: bool,
) -> terminal_support::Terminal {
    let mut command = portable_pty::CommandBuilder::new(binary);
    command.cwd(config.parent().expect("workspace"));
    command.arg("--config");
    command.arg(config);
    command.args(["--alt-screen", "--approval-mode", "full-access"]);
    if with_worker {
        command.arg("--worker-required");
    }
    command.arg("tui");
    command.env("HOME", home.path());
    command.env("COLOSSUS_HOME", home.colossus_home());
    command.env("COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY", JOURNAL_KEY);
    command.env("COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY", SIGNING_KEY);
    command.env("COLOSSUS_PROVIDER_TERMINAL_API_KEY", "terminal-secret");
    #[cfg(windows)]
    {
        command.env("USERPROFILE", home.path());
        command.env("LOCALAPPDATA", home.local_app_data());
        command.env("TEMP", home.temporary_directory());
        command.env("TMP", home.temporary_directory());
    }
    let terminal = terminal_support::Terminal::start(command);
    terminal.wait("Enter sends");
    terminal
}

fn open_picker(terminal: &terminal_support::Terminal) {
    terminal.send(b"/models\r");
    terminal.wait("Models · 1 configured");
}

fn expand_live(terminal: &terminal_support::Terminal) {
    terminal.send(b"\x1b[H\x1b[B\x1b[B");
    terminal.wait("› ▸ live");
    terminal.send(b"\r");
}

#[test]
fn tui_model_catalog_loads_on_demand_and_routes_edited_limits_in_both_hosts() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    for with_worker in [false, true] {
        let directory = tempdir().expect("workspace");
        let home = process_support::isolated_user_home(directory.path());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider");
        listener.set_nonblocking(true).expect("bounded accept");
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let config = write_doctor_config(directory.path(), &origin);
        let mut document: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        let mut unused = document["providers"]["profiles"]["live"].clone();
        unused["baseUrl"] = json!(format!("{origin}/unused"));
        document["providers"]["profiles"]["unused"] = unused;
        let config_bytes = serde_json::to_vec_pretty(&document).unwrap();
        fs::write(&config, &config_bytes).unwrap();
        let (dispatched, first_request) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut catalog = accept_until(&listener, Instant::now() + Duration::from_secs(30));
            let request = read_request(&mut catalog);
            dispatched
                .send(request.lines().next().unwrap().to_owned())
                .unwrap();
            held.recv_timeout(Duration::from_secs(10))
                .expect("release catalog");
            respond_json(&mut catalog, "200 OK", CATALOG);
            let mut turn = accept_until(&listener, Instant::now() + Duration::from_secs(30));
            let request = read_request(&mut turn);
            assert!(
                request.starts_with("POST /v1/chat/completions "),
                "{request:?}"
            );
            let (_, body) = request.split_once("\r\n\r\n").unwrap();
            let body: Value = serde_json::from_str(body).expect("generation request");
            assert_eq!(body["model"], "catalog-new");
            assert_eq!(body["max_tokens"], 2048);
            if body["stream"] == true {
                respond_sse(&mut turn, STREAM);
            } else {
                respond_json(&mut turn, "200 OK", RESPONSE);
            }
        });
        let worker_process = || {
            let mut command = goal_usage_process(binary, &config, &home);
            command.env("COLOSSUS_PROVIDER_TERMINAL_API_KEY", "terminal-secret");
            command
        };
        let mut worker = with_worker.then(|| {
            let process = worker_process()
                .arg("worker")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start worker");
            let worker = ChildGuard(process);
            let deadline = Instant::now() + Duration::from_secs(10);
            while !worker_process()
                .args(["worker", "--status"])
                .output()
                .unwrap()
                .status
                .success()
            {
                assert!(Instant::now() < deadline, "worker did not start");
                thread::sleep(Duration::from_millis(20));
            }
            worker
        });
        let terminal = catalog_terminal(binary, &config, &home, with_worker);
        open_picker(&terminal);
        terminal.wait("live · terminal-model");
        terminal.wait("▸ unused");
        assert!(matches!(
            first_request.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        expand_live(&terminal);
        assert_eq!(
            first_request.recv_timeout(Duration::from_secs(5)).unwrap(),
            "GET /v1/models HTTP/1.1"
        );
        terminal.wait("live · loading");
        terminal.send(b"\x1b");
        terminal.wait("Enter sends");
        open_picker(&terminal);
        expand_live(&terminal);
        terminal.wait("live · loading");
        release.send(()).unwrap();
        terminal.wait("catalog-new");
        terminal.send(b"\x1b[B\r");
        terminal.wait("Configure model");
        terminal.wait("65536");
        terminal.wait("8192");
        terminal.send(b"\t\x1b[3~2048");
        terminal.wait("2048");
        terminal.send(b"\r");
        terminal.wait_for_composer("picker applied", str::is_empty);
        terminal.command("Try the discovered model.", "DISCOVERED MODEL REPLIED");
        server.join().expect("provider requests");
        assert_eq!(fs::read(&config).unwrap(), config_bytes);
        if let Some(worker) = worker.as_mut() {
            drop(terminal);
            assert!(
                worker_process()
                    .args(["worker", "--shutdown"])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            wait_for_exit(&mut worker.0);
        }
    }
}
