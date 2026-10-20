use super::*;

#[cfg(windows)]
struct PrivateTestDirectory(std::path::PathBuf);

#[cfg(windows)]
impl Drop for PrivateTestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[cfg(not(windows))]
type PrivateTestDirectory = tempfile::TempDir;

fn private_root() -> (PrivateTestDirectory, ConfinedRoot) {
    #[cfg(not(windows))]
    let temporary = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let temporary = {
        let path = colossus_windows_native::current_user_profile_directory()
            .unwrap()
            .join(format!("colossus-logging-test-{}", uuid::Uuid::now_v7()));
        colossus_windows_native::create_private_directory(&path).unwrap();
        PrivateTestDirectory(path)
    };
    #[cfg(windows)]
    let path = &temporary.0;
    #[cfg(not(windows))]
    let path = temporary.path();
    let home = colossus_home::ColossusHome::ensure_at(path.join("home")).unwrap();
    let root = ConfinedRoot::bind(home.root()).unwrap();
    (temporary, root)
}

#[test]
fn rotation_is_bounded_and_a_second_writer_cannot_truncate_live_logs() {
    let (_temporary, root) = private_root();
    let mut writer = RotatingWriter::open(&root, "desktop", 128).unwrap();
    writer.write_all(b"first\n").unwrap();
    assert!(RotatingWriter::open(&root, "desktop", 128).is_err());
    for _ in 0..20 {
        writer.write_all(&[b'a'; 100]).unwrap();
    }
    let directory = root.path().join("logs");
    assert_eq!(
        std::fs::read_dir(&directory).unwrap().count(),
        LOG_FILES + 1
    );
    for slot in 0..LOG_FILES {
        assert!(
            std::fs::metadata(directory.join(format!("desktop-{slot}.jsonl")))
                .unwrap()
                .len()
                <= 128
        );
    }
    drop(writer);
    assert!(RotatingWriter::open(&root, "desktop", 128).is_ok());
}

#[test]
fn local_logs_include_owned_debug_events_but_never_journal_or_wire_payloads() {
    let (_temporary, root) = private_root();
    let sink = LocalLogSink::open(&root, "desktop", LocalLogLevel::Debug).unwrap();
    let level = sink.level;
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .with_span_list(false)
            .with_writer(sink.writer.clone())
            .with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
                level.accepts(metadata)
            })),
    );
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("private", secret = "private parent field");
        let _entered = span.enter();
        tracing::info!(target: "colossus.desktop", event_name = "desktop.started");
        tracing::debug!(target: "colossus.provider", event_name = "provider.connected", http_status = 200);
        tracing::info!(target: "colossus.journal", payload = "private journal payload");
        tracing::error!(target: "reqwest", payload = "private authorization header");
    });
    drop(sink);
    let text = std::fs::read_to_string(root.path().join("logs/desktop-0.jsonl")).unwrap();
    let records = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert!(text.contains("provider.connected"));
    assert!(!text.contains("private"));
    assert_eq!(
        LocalLogLevel::from_value(Some("debug")),
        LocalLogLevel::Debug
    );
    assert_eq!(
        LocalLogLevel::from_value(Some("reqwest=trace")),
        LocalLogLevel::Info
    );
}

#[test]
fn linked_log_files_are_refused_without_overwriting_the_target() {
    let (_temporary, root) = private_root();
    let directory = root.prepare_directory(Path::new("logs")).unwrap();
    let victim = root.open_file(Path::new("victim")).unwrap();
    victim.file().write_all(b"unchanged").unwrap();
    std::fs::hard_link(victim.path(), directory.join("desktop-0.jsonl")).unwrap();
    assert!(RotatingWriter::open(&root, "desktop", MAX_FILE_BYTES).is_err());
    assert_eq!(std::fs::read(victim.path()).unwrap(), b"unchanged");
}
