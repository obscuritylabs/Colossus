//! Native checkpoints for the fixture's own console; no desktop or other process is observed.
use super::*;
use colossus_windows_native::{ConsoleObserver, ConsoleSnapshot};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const RAW_INPUT_FLAGS: u64 = 0x0001 | 0x0002 | 0x0004;
const CAPTURE_FLAGS: u64 = 0x0010 | 0x0008 | 0x0080;
const QUICK_EDIT: u64 = 0x0040;
const ENVIRONMENT: &str = "COLOSSUS_TUI_NATIVE_CONSOLE_PROBE";
pub(super) const ORIGINAL_SENTINEL: &str = "native-console-original-sentinel";

pub(super) struct Probe {
    directory: tempfile::TempDir,
    next: Cell<u32>,
    main: RefCell<Option<Value>>,
}

impl Probe {
    pub fn new(command: &mut CommandBuilder) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("colossus-console-probe-")
            .tempdir()
            .expect("native probe directory");
        command.env(ENVIRONMENT, directory.path());
        Self {
            directory,
            next: Cell::new(0),
            main: RefCell::new(None),
        }
    }

    fn read(&self, file: &str) -> Value {
        let path = self.directory.path().join(file);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(error) = fs::read_to_string(self.directory.path().join("error.txt")) {
                panic!("native console recorder failed: {error}");
            }
            if let Ok(metadata) = fs::metadata(&path) {
                assert!(metadata.len() <= 1_048_576, "bounded native checkpoint");
                if let Ok(file) = fs::File::open(&path) {
                    let mut bytes = Vec::new();
                    file.take(1_048_577)
                        .read_to_end(&mut bytes)
                        .expect("read native checkpoint");
                    assert!(bytes.len() <= 1_048_576, "bounded native checkpoint read");
                    return serde_json::from_slice(&bytes).expect("native checkpoint JSON");
                }
            }
            assert!(
                Instant::now() < deadline,
                "native checkpoint did not arrive: {file}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn observe(&self) -> Value {
        let id = self.next.get() + 1;
        assert!(id <= 128, "bounded native checkpoints");
        self.next.set(id);
        fs::write(self.directory.path().join("request"), id.to_string())
            .expect("native checkpoint request");
        self.read(&format!("response-{id}.json"))
    }

    pub fn captured(&self) {
        let snapshot = self.observe();
        assert_capture_mode(&snapshot);
    }

    pub fn released(&self) {
        let snapshot = self.observe();
        let before = self.read("before.json");
        assert_eq!(
            mode(&snapshot),
            mode(&before) & !RAW_INPUT_FLAGS,
            "selection must restore the original native input flags while retaining raw input"
        );
    }

    pub fn alternate(&self, marker: &str) {
        let snapshot = self.observe();
        assert_capture_mode(&snapshot);
        assert!(
            screen(&snapshot, "active").contains(marker),
            "native active buffer lacks {marker:?}: {snapshot}"
        );
        assert_ne!(
            snapshot["active"],
            self.read("before.json")["active"],
            "transient screen must differ from original shell snapshot"
        );
    }

    pub fn checkpoint_main(&self) {
        *self.main.borrow_mut() = Some(self.observe());
    }

    pub fn main_screen(&self, notice: Option<&str>) {
        let initial = self.read("before.json");
        let deadline = Instant::now() + Duration::from_secs(5);
        // Mode restoration precedes the normal frame's repaint. Observe that
        // completed native frame, not the temporarily cleared app viewport.
        let snapshot = loop {
            let snapshot = self.observe();
            if mode(&snapshot) == mode(&initial) & !RAW_INPUT_FLAGS
                && screen(&snapshot, "active").contains("Message · Enter sends")
            {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "native main frame was not restored: {snapshot}"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let main = self.main.borrow();
        let main = main.as_ref().expect("pre-overlay checkpoint");
        let original = history_rows(main);
        let restored = history_rows(&snapshot);
        assert_eq!(
            restored.len(),
            original.len(),
            "native history viewport geometry changed"
        );
        if let Some(notice) = notice {
            // Cancellation appends a command title, its message and the composer
            // gap. Exactly those three rows move older history into scrollback;
            // the parent also checks every durable row there without omissions.
            let preserved = original.len().checked_sub(3).expect("native history rows");
            assert_eq!(
                restored[..preserved],
                original[3..],
                "native history changed beyond the expected cancellation notice scroll"
            );
            assert_eq!(restored[preserved].trim(), "› Command");
            assert_eq!(restored[preserved + 1].trim(), notice);
            assert!(
                restored[preserved + 2].trim().is_empty(),
                "composer gap missing after notice"
            );
            assert_eq!(
                snapshot["cursor"], main["cursor"],
                "cancelled overlay did not restore the composer cursor"
            );
        } else {
            assert_eq!(
                restored, original,
                "native history viewport changed after closing completion"
            );
            let cursor = &snapshot["cursor"];
            assert_eq!(
                cursor[0], main["cursor"][0],
                "completion moved composer row"
            );
            assert_eq!(
                cursor[1].as_u64(),
                main["cursor"][1].as_u64().map(|column| column + 1),
                "completion must retain the typed slash and its cursor"
            );
        }
        for marker in ["Choose theme", "Resume session", "Commands ·"] {
            assert!(
                !screen(&snapshot, "active").contains(marker),
                "transient content remained after closing: {marker}"
            );
        }
    }

    pub fn restored(&self, original_unchanged: bool) {
        let before = self.read("before.json");
        let after = self.read("after.json");
        assert_eq!(
            mode(&after),
            mode(&before),
            "native input mode was not restored on exit"
        );
        assert!(
            screen(&before, "active").contains(ORIGINAL_SENTINEL),
            "original shell sentinel was not seeded"
        );
        if original_unchanged {
            assert_eq!(
                after["active"], before["active"],
                "full-screen TUI did not restore original native screen content"
            );
            assert_eq!(
                after["cursor"], before["cursor"],
                "full-screen TUI did not restore original native cursor"
            );
        }
    }

    pub fn original_sentinel_preserved(&self) {
        let after = self.read("after.json");
        assert!(
            screen(&after, "active").contains(ORIGINAL_SENTINEL),
            "original shell sentinel did not survive: {after}"
        );
    }
}

fn history_rows(snapshot: &Value) -> Vec<&str> {
    snapshot["active"]
        .as_array()
        .expect("native rows")
        .iter()
        .map(|row| row.as_str().expect("native row"))
        .take_while(|row| !row.contains("Message · Enter sends"))
        .collect()
}

fn mode(snapshot: &Value) -> u64 {
    snapshot["input_mode"].as_u64().expect("native input flags")
}

fn screen(snapshot: &Value, key: &str) -> String {
    snapshot[key]
        .as_array()
        .expect("native rows")
        .iter()
        .map(|row| row.as_str().expect("native row"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_capture_mode(snapshot: &Value) {
    let flags = mode(snapshot);
    assert_eq!(
        flags & CAPTURE_FLAGS,
        CAPTURE_FLAGS,
        "mouse/window capture is not enabled: {snapshot}"
    );
    assert_eq!(
        flags & (RAW_INPUT_FLAGS | QUICK_EDIT),
        0,
        "capture must use raw input without QuickEdit: {snapshot}"
    );
}

pub(super) struct Recorder {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Recorder {
    pub fn start() -> Option<Self> {
        let directory = std::env::var_os(ENVIRONMENT).map(PathBuf::from)?;
        // Distinct original content makes a cleared-but-active alternate buffer fail restoration checks.
        println!("{ORIGINAL_SENTINEL}");
        std::io::stdout()
            .flush()
            .expect("flush original console sentinel");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (ready, started) = std::sync::mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let outcome = record(&directory, &worker_stop, &ready);
            if let Err(error) = outcome {
                let _ = fs::write(directory.join("error.txt"), &error);
                let _ = ready.send(Err(error));
            }
        });
        // Own shutdown before awaiting startup so a timeout also stops and joins the worker.
        let recorder = Self {
            stop,
            worker: Some(worker),
        };
        started
            .recv_timeout(Duration::from_secs(5))
            .expect("start native recorder")
            .expect("open native console");
        Some(recorder)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn record(
    directory: &Path,
    stop: &AtomicBool,
    ready: &std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let observer = ConsoleObserver::current().map_err(|error| error.to_string())?;
    save(
        directory,
        "before.json",
        observer.snapshot().map_err(|error| error.to_string())?,
    )?;
    ready.send(Ok(())).map_err(|error| error.to_string())?;
    let mut previous = 0;
    while !stop.load(Ordering::Acquire) {
        let request = fs::read_to_string(directory.join("request"))
            .ok()
            .and_then(|value| value.parse::<u32>().ok());
        if let Some(id) = request.filter(|id| *id > previous && *id <= 128) {
            save(
                directory,
                &format!("response-{id}.json"),
                observer.snapshot().map_err(|error| error.to_string())?,
            )?;
            previous = id;
        }
        thread::sleep(Duration::from_millis(5));
    }
    save(
        directory,
        "after.json",
        observer.snapshot().map_err(|error| error.to_string())?,
    )
}

fn save(directory: &Path, name: &str, snapshot: ConsoleSnapshot) -> Result<(), String> {
    let bytes = serde_json::to_vec(&json!({ "input_mode":snapshot.input_mode,
        "active":snapshot.active.rows,
        "cursor":[snapshot.active.cursor_row,snapshot.active.cursor_column] }))
    .map_err(|error| error.to_string())?;
    let staged = directory.join(format!("{name}.tmp"));
    fs::write(&staged, bytes).map_err(|error| error.to_string())?;
    fs::rename(staged, directory.join(name)).map_err(|error| error.to_string())
}
