//! Disposable loopback TLS fixture, never an operator/default certificate store.
use super::*;
use std::{path::Path, process::Stdio};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub(super) struct Fixture {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    allocation: PathBuf,
    pub(super) material: PathBuf,
    pub(super) metadata: Value,
}
impl Fixture {
    pub(super) async fn start() -> Self {
        let allocation = private_tempdir().keep();
        let material = allocation.join("material");
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../native/browser/scripts/pki_fixture.py");
        let mut child = Command::new("/usr/bin/python3")
            .arg("-B")
            .arg(script)
            .arg("--directory")
            .arg(&material)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("start disposable fixture");
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let mut fixture = Self {
            child,
            input,
            output,
            allocation,
            material,
            metadata: Value::Null,
        };
        let ready = fixture.line().await;
        assert_eq!(ready["ready"], true);
        fixture.metadata = ready["fixture"].clone();
        assert_eq!(fixture.metadata["urls"].as_object().unwrap().len(), 6);
        for value in fixture.metadata["urls"].as_object().unwrap().values() {
            let value = value.as_str().unwrap();
            assert!(value.starts_with("https://127.0.0.1:"));
            BrowserOrigin::parse(value).unwrap();
        }
        fixture
    }
    pub(super) fn url(&self, endpoint: &str) -> String {
        self.metadata["urls"][endpoint].as_str().unwrap().into()
    }
    pub(super) fn fingerprint(&self, identity: &str) -> String {
        self.metadata["fingerprints_sha256"][identity]
            .as_str()
            .unwrap()
            .into()
    }
    pub(super) fn bytes(&self, leaf: &str, limit: usize) -> zeroize::Zeroizing<Vec<u8>> {
        use std::{
            fs::OpenOptions,
            io::Read as _,
            os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
        };
        assert!(!leaf.contains('/') && !leaf.contains('\\'));
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(self.material.join(leaf))
            .unwrap();
        let before = source.metadata().unwrap();
        assert!(
            before.is_file()
                && before.mode() & 0o077 == 0
                && before.nlink() == 1
                && before.uid() == std::fs::metadata(&self.allocation).unwrap().uid()
                && before.len() <= limit as u64
        );
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        source
            .by_ref()
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        let after = source.metadata().unwrap();
        assert!(
            bytes.len() <= limit
                && before.dev() == after.dev()
                && before.ino() == after.ino()
                && before.len() == after.len()
                && before.mtime() == after.mtime()
                && before.mtime_nsec() == after.mtime_nsec()
                && before.ctime() == after.ctime()
                && before.ctime_nsec() == after.ctime_nsec()
        );
        bytes
    }
    pub(super) async fn report(&mut self) -> Value {
        self.input.write_all(b"report\n").await.unwrap();
        let report = self.line().await;
        let report = report["report"].clone();
        assert_eq!(report["overflow"], false);
        assert!(report["events"].as_array().unwrap().len() <= 512);
        report
    }
    pub(super) async fn close(mut self, native_cleanup_confirmed: bool) {
        self.input.write_all(b"stop\n").await.unwrap();
        drop(self.input);
        assert!(
            tokio::time::timeout(Duration::from_secs(10), self.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        if native_cleanup_confirmed {
            std::fs::remove_dir_all(&self.allocation).unwrap();
        }
    }
    async fn line(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(60), async {
            let mut bytes = Vec::new();
            loop {
                let byte = self
                    .output
                    .read_u8()
                    .await
                    .expect("bounded fixture receipt");
                if byte == b'\n' {
                    return serde_json::from_slice(&bytes).unwrap();
                }
                assert!(bytes.len() < 128 * 1024);
                bytes.push(byte);
            }
        })
        .await
        .expect("fixture receipt deadline")
    }
}
