//! Exact developer-fixture child ownership and inherited private descriptors.
use colossus_browser_bridge::{
    BrowserBridgeDriver, BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel,
};
use colossus_browser_presentation::{PresentationChannel, PresentationClient};
use sha2::{Digest as _, Sha256};
use std::{
    io::Write as _,
    os::{
        fd::AsRawFd as _,
        unix::{
            fs::{DirBuilderExt as _, OpenOptionsExt as _},
            net::UnixStream,
            process::CommandExt as _,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub struct Launched {
    child: Child,
    root: PathBuf,
    pub browser: BrowserBridgeDriver,
    pub presentation: PresentationClient,
    pub digest: [u8; 32],
    reaped: bool,
}
impl Launched {
    pub async fn start(
        host: &Path,
        component: &Path,
        enrollment: BrowserBridgeEnrollment,
        port: u16,
        human_input: bool,
    ) -> Result<Self, &'static str> {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| "fixture randomness unavailable")?;
        let suffix = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let root = std::env::temp_dir().join(format!("colossus-presentation-{suffix}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(|_| "fixture private state unavailable")?;
        let profile = root.join("profile");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&profile)
            .map_err(|_| "fixture private profile unavailable")?;
        let mut pairs = (0..4)
            .map(|_| UnixStream::pair())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "fixture private channels unavailable")?;
        let descriptors = pairs
            .iter()
            .map(|(_, child)| child.as_raw_fd())
            .collect::<Vec<_>>();
        let mut command = Command::new(host);
        command
            .args(descriptors.iter().map(i32::to_string))
            .current_dir(&root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &root)
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        // Explicit fixture diagnostics contain no real profile, site or PKI data.
        // Browser bootstrap key/credentials remain on the inherited private pipe.
        let diagnostic = if std::env::var_os("COLOSSUS_PRESENTATION_PROBE_DIAGNOSTICS").as_deref()
            == Some(std::ffi::OsStr::new("1"))
        {
            let path = root.join("startup-diagnostic");
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| "fixture private diagnostics unavailable")?;
            command.stderr(Stdio::from(file));
            Some(path)
        } else {
            None
        };
        #[cfg(target_os = "linux")]
        command.env("LD_LIBRARY_PATH", component);
        #[cfg(not(target_os = "linux"))]
        let _ = component; // The staged macOS entry owns its scoped framework loader.
        // SAFETY: the child has no Rust threads; only these four exact owned sockets
        // are made inheritable, and native bootstrap closes every unrelated fd.
        unsafe {
            command.pre_exec(move || {
                for descriptor in &descriptors {
                    if libc::fcntl(*descriptor, libc::F_SETFD, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut pending_child = PendingChild(Some(
            command
                .spawn()
                .map_err(|_| "fixture native host unavailable")?,
        ));
        let mut raw_key = Zeroizing::new([0_u8; 32]);
        getrandom::fill(raw_key.as_mut()).map_err(|_| "fixture key unavailable")?;
        let digest: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&enrollment).map_err(|_| "fixture enrollment invalid")?,
        )
        .into();
        let key = BrowserBridgeKey::from_bootstrap(Zeroizing::new(*raw_key));
        let presentation_key = key.derive_presentation_key();
        let configuration = serde_json::to_vec(&serde_json::json!({"enrollment":enrollment,"profile_path":profile,"proxy":{"address":"127.0.0.1","port":port,"username":"colossus","password":super::fixture::PASSWORD},"presentation":{"human_input":human_input}})).map_err(|_| "fixture bootstrap invalid")?;
        let (mut bootstrap, child_bootstrap) = pairs.remove(0);
        drop(child_bootstrap);
        bootstrap
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| "fixture bootstrap unavailable")?;
        let initialized = (|| {
            bootstrap.write_all(raw_key.as_ref())?;
            bootstrap.write_all(&(configuration.len() as u32).to_be_bytes())?;
            bootstrap.write_all(&configuration)
        })();
        bootstrap
            .set_nonblocking(true)
            .map_err(|_| "fixture bootstrap receipt unavailable")?;
        let bootstrap = tokio::net::UnixStream::from_std(bootstrap)
            .map_err(|_| "fixture bootstrap receipt unavailable")?;
        let channels = pairs
            .into_iter()
            .map(|(parent, child)| {
                drop(child);
                parent.set_nonblocking(true)?;
                tokio::net::UnixStream::from_std(parent)
            })
            .collect::<Result<Vec<_>, std::io::Error>>();
        let result = async {
            initialized.map_err(|_| "fixture bootstrap failed")?;
            super::receipt::initialized(bootstrap).await?;
            let mut channels = channels
                .map_err(|_| "fixture channel setup failed")?
                .into_iter();
            let (reader, writer) = channels.next().ok_or("fixture data absent")?.into_split();
            let data = InheritedBrowserChannel::new(reader, writer);
            let (reader, writer) = channels
                .next()
                .ok_or("fixture control absent")?
                .into_split();
            let control = InheritedBrowserChannel::new(reader, writer);
            let browser = BrowserBridgeDriver::connect(data, control, enrollment, key)
                .await
                .map_err(|_| "fixture browser readiness failed")?;
            let (reader, writer) = channels
                .next()
                .ok_or("fixture presentation absent")?
                .into_split();
            let presentation = PresentationClient::connect(
                PresentationChannel::new(reader, writer),
                presentation_key,
                digest,
            )
            .await
            .map_err(|_| "fixture presentation readiness failed")?;
            Ok((browser, presentation))
        }
        .await;
        match result {
            Ok((browser, presentation)) => Ok(Self {
                child: pending_child
                    .0
                    .take()
                    .ok_or("fixture native ownership missing")?,
                root,
                browser,
                presentation,
                digest,
                reaped: false,
            }),
            Err(error) => Err(super::receipt::failure_category(
                error,
                diagnostic.as_deref(),
            )),
        }
    }
    pub async fn finish(&mut self) -> Result<(), &'static str> {
        self.presentation.disconnect();
        self.browser.disconnect_for_shutdown();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|_| "fixture native exit unknown")?
            {
                self.reaped = true;
                if !status.success() {
                    return Err("fixture CEF shutdown failed");
                }
                process_group_quiet(self.child.id()).await?;
                std::fs::remove_dir_all(&self.root)
                    .map_err(|_| "fixture private state cleanup failed")?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("fixture native shutdown timed out");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

async fn process_group_quiet(group: u32) -> Result<(), &'static str> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // SAFETY: read-only group existence check; no signal or identity-based
        // cleanup is issued after the retained child was positively reaped.
        if unsafe { libc::killpg(group as i32, 0) } < 0 {
            return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                Ok(())
            } else {
                Err("fixture native group exit unknown")
            };
        }
        if Instant::now() >= deadline {
            return Err("fixture native helpers remain");
        }
        // Container init may reap an already exited helper just after its parent.
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
struct PendingChild(Option<Child>);
impl Drop for PendingChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.take() {
            reap_failed(child);
        }
    }
}
fn reap_failed(mut child: Child) {
    // SAFETY: unreaped exactly spawned child reserves the process-group identity;
    // this developer fixture never signals an image/PID nominated by a renderer.
    let _ = unsafe { libc::killpg(child.id() as i32, libc::SIGKILL) };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
impl Drop for Launched {
    fn drop(&mut self) {
        self.presentation.disconnect();
        self.browser.disconnect_for_shutdown();
        if !self.reaped {
            // SAFETY: this child remains unreaped and exactly owns the created group.
            let _ = unsafe { libc::killpg(self.child.id() as i32, libc::SIGKILL) };
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if self.child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        // Failure preserves the exact created state for operator reconciliation.
    }
}
