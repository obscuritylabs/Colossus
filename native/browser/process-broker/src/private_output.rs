//! Fixed compiler-sealed private evidence, absent from production and native IPC.
use crate::{ProcessReceipt, arguments::invalid, custody_markers, relay::OutputDrains};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write as _},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

pub(crate) struct Diagnostic {
    root: PathBuf,
    namespace: PathBuf,
    namespace_fd: File,
    directory: File,
}

#[cfg(test)]
#[path = "private_output_tests.rs"]
mod tests;
impl Diagnostic {
    pub(crate) fn open(main: &Path, policy: [u8; 32], compiled: &str) -> io::Result<Self> {
        let (namespace, namespace_fd) = custody_markers::namespace(main, policy)?;
        let root = PathBuf::from(compiled);
        if root != namespace.join("process-diagnostics") || root.canonicalize()? != root {
            return Err(invalid("diagnostic path is outside exact policy namespace"));
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC)
                    .bits() as i32,
            )
            .open(&root)?;
        let diagnostic = Self {
            root,
            namespace,
            namespace_fd,
            directory,
        };
        diagnostic.revalidate()?;
        Ok(diagnostic)
    }
    fn revalidate(&self) -> io::Result<()> {
        let parent = self.namespace_fd.metadata()?;
        let namespace = self.namespace.symlink_metadata()?;
        let held = self.directory.metadata()?;
        let path = self.root.symlink_metadata()?;
        if !parent.is_dir()
            || !namespace.is_dir()
            || !held.is_dir()
            || !path.is_dir()
            || parent.mode() & 0o7777 != 0o700
            || held.mode() & 0o7777 != 0o700
            || (parent.dev(), parent.ino(), parent.uid())
                != (namespace.dev(), namespace.ino(), namespace.uid())
            || (held.dev(), held.ino(), held.uid()) != (path.dev(), path.ino(), parent.uid())
        {
            return Err(invalid("private process diagnostic directory changed"));
        }
        Ok(())
    }
    pub(crate) fn persist(
        &self,
        output: Option<&OutputDrains>,
        receipt: &ProcessReceipt,
    ) -> io::Result<()> {
        let captured = match output {
            Some(output) => output.captured()?,
            None => vec![Vec::new(), Vec::new()],
        };
        if captured.len() != 2 || captured.iter().any(|value| value.len() > 65536) {
            return Err(invalid("private Main output exceeded its fixed bound"));
        }
        for (name, bytes) in ["main.stdout.log", "main.stderr.log"]
            .into_iter()
            .zip(captured)
        {
            self.write(name, &bytes)?;
        }
        self.write(
            "creator-cleanup.json",
            &serde_json::to_vec(receipt).map_err(io::Error::other)?,
        )?;
        self.directory.sync_all()?;
        self.revalidate()
    }
    fn write(&self, name: &'static str, bytes: &[u8]) -> io::Result<()> {
        self.revalidate()?;
        let handle = rustix::fs::openat(
            &self.directory,
            name,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )?;
        let mut output = File::from(handle);
        let metadata = output.metadata()?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.mode() & 0o7777 != 0o600
            || metadata.uid() != self.namespace_fd.metadata()?.uid()
        {
            return Err(invalid("private process diagnostic file is not exclusive"));
        }
        output.write_all(bytes)?;
        output.sync_all()?;
        self.revalidate()
    }
}
