//! Fresh private NSS HOME. Never discovers or mutates an existing user's trust/key store.
use crate::{
    identity::{Binding, Policy},
    tooling,
};
use colossus_native_browser_pki::{ca_der, fingerprint};
use colossus_ports::BrowserDriverError;
use serde::Deserialize;
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{Read as _, Write as _},
    os::{
        fd::AsRawFd as _,
        unix::fs::{
            DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    path: PathBuf,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pfx_file: PathBuf,
    passphrase_file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    require_source_retirement: bool,
    certutil: Tool,
    pk12util: Tool,
    ca_files: Vec<PathBuf>,
    identities: Vec<Identity>,
    bindings: Vec<Binding>,
}

impl Configuration {
    pub fn confirm_source_retirement(&self) -> Result<(), BrowserDriverError> {
        if !self.require_source_retirement {
            return Err(BrowserDriverError::Denied);
        }
        for identity in &self.identities {
            for path in [&identity.pfx_file, &identity.passphrase_file] {
                private_directory(path.parent().ok_or(BrowserDriverError::Denied)?)?;
                match std::fs::symlink_metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    _ => return Err(BrowserDriverError::Denied),
                }
            }
        }
        Ok(())
    }
}

/// Positively created unique directory, owned until native CEF shutdown.
pub struct Home {
    path: PathBuf,
    parent: File,
    directory: Option<File>,
    created_identity: Option<std::fs::Metadata>,
    name: CString,
    quarantine: Option<CString>,
    ownership_unknown: bool,
    removed: bool,
}
impl Drop for Home {
    fn drop(&mut self) {
        if self.cleanup().is_err() {
            // Fixed native diagnostic only. An unknown replacement is retained;
            // a pathname alone never grants permission to remove another tree.
            eprintln!("browser outcome unknown");
        }
    }
}
impl Home {
    pub fn finish(mut self) -> Result<(), BrowserDriverError> {
        self.cleanup()
    }

    fn create(parent_path: &Path) -> Result<Self, BrowserDriverError> {
        let mut home = Self::allocate(parent_path)?;
        home.bind()?;
        Ok(home)
    }

    fn allocate(parent_path: &Path) -> Result<Self, BrowserDriverError> {
        private_directory(parent_path)?;
        let expected =
            std::fs::symlink_metadata(parent_path).map_err(|_| BrowserDriverError::Denied)?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent_path)
            .map_err(|_| BrowserDriverError::Denied)?;
        let opened = parent.metadata().map_err(|_| BrowserDriverError::Denied)?;
        if opened.dev() != expected.dev() || opened.ino() != expected.ino() {
            return Err(BrowserDriverError::Denied);
        }
        let name = random_name("colossus-nss-")?;
        // SAFETY: retained private parent directory, generated slash-free NUL-
        // terminated basename, and private directory mode. Success means fresh.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(BrowserDriverError::Denied);
        }
        let path = parent_path.join(name.to_str().expect("generated ASCII basename"));
        // Register the created entry before any fallible descriptor binding.
        // A pending owner only removes its verified empty directory; it cannot
        // recursively prune entries added before native ownership is bound.
        let mut home = Self {
            path,
            parent,
            directory: None,
            created_identity: None,
            name,
            quarantine: None,
            ownership_unknown: false,
            removed: false,
        };
        let identity = std::fs::symlink_metadata(home.entry_path(&home.name))
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        home.created_identity = Some(identity.clone());
        // SAFETY: geteuid observes effective identity and takes no arguments.
        let uid = unsafe { libc::geteuid() };
        if !identity.is_dir() || identity.uid() != uid || identity.mode() & 0o077 != 0 {
            home.ownership_unknown = true;
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(home)
    }

    fn bind(&mut self) -> Result<(), BrowserDriverError> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(self.entry_path(&self.name))
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let identity = directory
            .metadata()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let created = self
            .created_identity
            .as_ref()
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        if !identity.is_dir()
            || identity.uid() != created.uid()
            || identity.mode() & 0o077 != 0
            || identity.dev() != created.dev()
            || identity.ino() != created.ino()
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        self.directory = Some(directory);
        Ok(())
    }

    fn entry_path(&self, name: &CString) -> PathBuf {
        PathBuf::from(format!(
            "/proc/self/fd/{}/{}",
            self.parent.as_raw_fd(),
            name.to_str().expect("generated ASCII basename")
        ))
    }

    fn cleanup(&mut self) -> Result<(), BrowserDriverError> {
        if self.ownership_unknown {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        if self.removed {
            return self
                .parent
                .sync_all()
                .map_err(|_| BrowserDriverError::OutcomeUnknown);
        }
        let held = match &self.directory {
            Some(directory) => directory
                .metadata()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
            None => {
                let Some(identity) = &self.created_identity else {
                    self.ownership_unknown = true;
                    return Err(BrowserDriverError::OutcomeUnknown);
                };
                identity.clone()
            }
        };
        if self.quarantine.is_none() {
            let name = random_name(".retired-colossus-nss-")
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            // SAFETY: retained private parent and generated basenames. NOREPLACE
            // never overwrites an existing destination; verify the moved inode.
            if unsafe {
                libc::renameat2(
                    self.parent.as_raw_fd(),
                    self.name.as_ptr(),
                    self.parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } != 0
            {
                self.ownership_unknown = true;
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            self.quarantine = Some(name);
        }
        let name = self.quarantine.as_ref().expect("retirement name retained");
        let path = self.entry_path(name);
        let moved =
            std::fs::symlink_metadata(&path).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if !moved.is_dir()
            || moved.uid() != held.uid()
            || moved.mode() & 0o077 != 0
            || moved.dev() != held.dev()
            || moved.ino() != held.ino()
        {
            self.ownership_unknown = true;
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        if self.directory.is_some() {
            std::fs::remove_dir_all(path).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        } else {
            // SAFETY: retained parent and generated quarantine basename; the
            // moved inode was verified. Pending ownership allows empty rmdir
            // only, requiring no newly opened fd even if binding hit EMFILE.
            if unsafe { libc::unlinkat(self.parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) }
                != 0
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        // Set the fence before sync: Drop may retry sync but must never remove a
        // future entry that reuses an already retired basename after success.
        self.removed = true;
        self.parent
            .sync_all()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)
    }
    pub fn prepare(
        profile: &Path,
        configuration: Option<&Configuration>,
    ) -> Result<(Self, Policy), BrowserDriverError> {
        let parent = profile.parent().ok_or(BrowserDriverError::Denied)?;
        let home = Self::create(parent)?;
        // SAFETY: bootstrap runs on the only thread before CEF, NSS utility threads, and Tokio.
        // Helpers inherit this newly owned HOME; no default personal store is consulted.
        unsafe {
            std::env::set_var("HOME", &home.path);
            std::env::set_var("XDG_CONFIG_HOME", home.path.join(".config"));
            std::env::set_var("XDG_DATA_HOME", home.path.join(".local/share"));
        }
        let Some(configuration) = configuration else {
            return Ok((home, Policy::default()));
        };
        if !configuration.require_source_retirement {
            return Err(BrowserDriverError::Denied);
        }
        let policy = Policy::new(configuration.bindings.clone())?;
        if configuration.ca_files.len() > 32 || configuration.identities.len() > 32 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        home.import(configuration)?;
        Ok((home, policy))
    }

    fn import(&self, configuration: &Configuration) -> Result<(), BrowserDriverError> {
        let (certutil, pk12util, _native_libraries) = tooling::tools(
            &configuration.certutil.path,
            &configuration.certutil.sha256,
            &configuration.pk12util.path,
            &configuration.pk12util.sha256,
        )?;
        let pki = self.path.join(".pki");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&pki)
            .map_err(|_| BrowserDriverError::Denied)?;
        let db = pki.join("nssdb");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&db)
            .map_err(|_| BrowserDriverError::Denied)?;
        let database = format!("sql:{}", db.display());
        run(
            &certutil,
            &self.path,
            &["-N", "--empty-password", "-d", &database],
        )?;
        for (index, identity) in configuration.identities.iter().enumerate() {
            let pfx = private_input(&identity.pfx_file, 4 * 1024 * 1024)?;
            let password = private_input(&identity.passphrase_file, 128)?;
            validate_password(&password)?;
            let pfx_path = self.path.join(format!("input-{index}.pfx"));
            let password_path = self.path.join(format!("password-{index}"));
            write_private(&pfx_path, &pfx)?;
            write_private(&password_path, &password)?;
            let result = run(
                &pk12util,
                &self.path,
                &[
                    "-i",
                    text(&pfx_path)?,
                    "-d",
                    &database,
                    "-w",
                    text(&password_path)?,
                ],
            );
            std::fs::remove_file(pfx_path).map_err(|_| BrowserDriverError::Failed)?;
            std::fs::remove_file(password_path).map_err(|_| BrowserDriverError::Failed)?;
            result?;
        }
        // PKCS#12 is identity enrollment, never authority to trust its CA chain.
        // Strip every imported explicit trust flag before adding reviewed CA roots separately.
        let listed = run(&certutil, &self.path, &["-L", "-d", &database])?;
        let listed = std::str::from_utf8(&listed).map_err(|_| BrowserDriverError::Denied)?;
        for line in listed
            .lines()
            .skip(4)
            .filter(|line| !line.trim().is_empty())
        {
            let line = line.trim_end();
            let split = line
                .rfind(char::is_whitespace)
                .ok_or(BrowserDriverError::Denied)?;
            let nickname = line[..split].trim_end();
            if nickname.is_empty()
                || nickname.len() > 1024
                || !nickname.bytes().all(|byte| (32..127).contains(&byte))
            {
                return Err(BrowserDriverError::Denied);
            }
            run(
                &certutil,
                &self.path,
                &["-M", "-d", &database, "-n", nickname, "-t", ",,"],
            )?;
        }
        for (index, path) in configuration.ca_files.iter().enumerate() {
            let bytes = private_input(path, 1024 * 1024)?;
            let der = ca_der(&bytes).map_err(|_| BrowserDriverError::Denied)?;
            let nickname = format!("colossus-ca-{}", fingerprint(&der));
            let input = self.path.join(format!("ca-{index}.der"));
            write_private(&input, &der)?;
            let result = run(
                &certutil,
                &self.path,
                &[
                    "-A",
                    "-d",
                    &database,
                    "-n",
                    &nickname,
                    "-t",
                    "C,,",
                    "-i",
                    text(&input)?,
                ],
            );
            std::fs::remove_file(input).map_err(|_| BrowserDriverError::Failed)?;
            result?;
        }
        // Chromium must see restrictive permissions even if an installed utility's umask differs.
        for entry in std::fs::read_dir(&db).map_err(|_| BrowserDriverError::Failed)? {
            let entry = entry.map_err(|_| BrowserDriverError::Failed)?;
            if !entry
                .file_type()
                .map_err(|_| BrowserDriverError::Failed)?
                .is_file()
            {
                return Err(BrowserDriverError::Denied);
            }
            std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o600))
                .map_err(|_| BrowserDriverError::Failed)?;
        }
        Ok(())
    }
}

fn random_name(prefix: &str) -> Result<CString, BrowserDriverError> {
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|_| BrowserDriverError::Unavailable)?;
    Ok(CString::new(format!("{prefix}{}", fingerprint(&nonce)))
        .expect("generated ASCII basename has no NUL"))
}

fn text(path: &Path) -> Result<&str, BrowserDriverError> {
    path.to_str().ok_or(BrowserDriverError::Denied)
}

fn validate_password(password: &[u8]) -> Result<(), BrowserDriverError> {
    // The accepted pinned-utility format is conservative: one printable ASCII
    // line, never a newline/Unicode conversion or an unproven long-reader truncation.
    if password.is_empty()
        || password.len() > 128
        || !password.iter().all(|byte| (32..=126).contains(byte))
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}
fn private_directory(path: &Path) -> Result<(), BrowserDriverError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| BrowserDriverError::Denied)?;
    // SAFETY: geteuid takes no arguments and observes current effective identity.
    let uid = unsafe { libc::geteuid() };
    if !path.is_absolute()
        || !metadata.is_dir()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || path
            .canonicalize()
            .map_err(|_| BrowserDriverError::Denied)?
            != path
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}
fn private_input(path: &Path, limit: u64) -> Result<Zeroizing<Vec<u8>>, BrowserDriverError> {
    private_directory(path.parent().ok_or(BrowserDriverError::Denied)?)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| BrowserDriverError::Denied)?;
    let metadata = file.metadata().map_err(|_| BrowserDriverError::Denied)?;
    // SAFETY: geteuid takes no arguments and observes current effective identity.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() == 0
        || metadata.len() > limit
    {
        return Err(BrowserDriverError::Denied);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    std::io::Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BrowserDriverError::Denied)?;
    if bytes.len() as u64 > limit {
        return Err(BrowserDriverError::Denied);
    }
    Ok(bytes)
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), BrowserDriverError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|_| BrowserDriverError::Failed)
}

fn run(
    tool: &File,
    home: &Path,
    arguments: &[&str],
) -> Result<Zeroizing<Vec<u8>>, BrowserDriverError> {
    // Execute the verified open ELF inode, not a mutable pathname. No password argv/env.
    let mut child = Command::new(format!("/proc/self/fd/{}", tool.as_raw_fd()))
        .args(arguments)
        .env_clear()
        .env("HOME", home)
        .env("LANG", "C")
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| BrowserDriverError::Unavailable)?;
    let stdout = child.stdout.take().ok_or(BrowserDriverError::Failed)?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        stdout
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let output = reader
        .join()
        .map_err(|_| BrowserDriverError::Failed)?
        .map_err(|_| BrowserDriverError::Failed)?;
    if !status.is_some_and(|status| status.success()) || output.len() > 128 * 1024 {
        return Err(BrowserDriverError::Failed);
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
