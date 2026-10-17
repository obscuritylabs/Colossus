//! Private native upload/download quarantine; never accepts a model filesystem path.
use crate::owned_directory::{OwnedDirectory, random_name};
use colossus_ports::BrowserDriverError;
use std::{
    ffi::CString,
    fs::{File, Metadata},
    io::{Read as _, Write as _},
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MAX_FILES: usize = 8;
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_STAGE_BYTES: usize = 32 * 1024 * 1024;

/// Native-only path and retained inode; deliberately has no Debug/Serialize implementation.
pub struct OwnedFile {
    path: PathBuf,
    _file: File,
}
impl OwnedFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// One issued download destination; consuming it prevents replay or cross-stage use.
pub struct Reservation {
    stage: CString,
    slot: usize,
    path: PathBuf,
}
impl Reservation {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

struct Slot {
    directory: OwnedDirectory,
    name: CString,
    file: Option<File>,
    pending: bool,
}

/// Retain until CefShutdown; cancellation never acknowledges file/directory removal.
pub struct Stage {
    directory: OwnedDirectory,
    nonce: CString,
    slots: Vec<Slot>,
    bytes: usize,
    shutdown_confirmed: bool,
}
impl Stage {
    pub fn create(profile: &Path) -> Result<Self, BrowserDriverError> {
        Ok(Self {
            directory: OwnedDirectory::create(profile, ".browser-transfers-")?,
            nonce: random_name("stage-")?,
            slots: Vec::new(),
            bytes: 0,
            shutdown_confirmed: false,
        })
    }

    pub fn upload(
        &mut self,
        bytes: &[u8],
        display_name: &str,
    ) -> Result<OwnedFile, BrowserDriverError> {
        self.capacity(bytes.len())?;
        let name = display_name_leaf(display_name)?;
        let directory = OwnedDirectory::create(self.directory.path(), "upload-")?;
        self.slots.push(Slot {
            directory,
            name,
            file: None,
            pending: false,
        });
        let slot = self.slots.last_mut().expect("inserted upload slot");
        let mut file = slot.directory.open_file(&slot.name, true)?;
        // Retain ownership before any fallible write/metadata operation.
        slot.file = Some(
            file.try_clone()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
        );
        file.write_all(bytes)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        file.sync_all()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        private_file(
            &file
                .metadata()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
            bytes.len(),
        )?;
        self.bytes += bytes.len();
        Ok(OwnedFile {
            path: slot.directory.path().join(display_name),
            _file: file,
        })
    }

    pub fn reserve_download(&mut self) -> Result<Reservation, BrowserDriverError> {
        self.capacity(0)?;
        let directory = OwnedDirectory::create(self.directory.path(), "download-")?;
        let path = directory.path().join("download");
        let index = self.slots.len();
        self.slots.push(Slot {
            directory,
            name: c"download".to_owned(),
            file: None,
            pending: true,
        });
        // Do not precreate the final inode: Chromium may atomically rename a
        // .crdownload sibling into this exact unique private destination.
        Ok(Reservation {
            stage: self.nonce.clone(),
            slot: index,
            path,
        })
    }

    pub fn completed(
        &mut self,
        reservation: Reservation,
        expected_size: u32,
    ) -> Result<Zeroizing<Vec<u8>>, BrowserDriverError> {
        let length = expected_size as usize;
        if reservation.stage != self.nonce
            || length > MAX_FILE_BYTES
            || self
                .bytes
                .checked_add(length)
                .is_none_or(|total| total > MAX_STAGE_BYTES)
        {
            return Err(BrowserDriverError::Denied);
        }
        self.directory.verify_namespace()?;
        let slot = self
            .slots
            .get_mut(reservation.slot)
            .ok_or(BrowserDriverError::Denied)?;
        if !slot.pending || reservation.path != slot.directory.path().join("download") {
            return Err(BrowserDriverError::Denied);
        }
        let mut file = slot.directory.open_file(&slot.name, false)?;
        let before = file
            .metadata()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        private_file(&before, length)?;
        // Retain the actual completed inode before reading quarantined bytes.
        slot.file = Some(
            file.try_clone()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
        );
        let mut bytes = Zeroizing::new(Vec::with_capacity(length));
        std::io::Read::by_ref(&mut file)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let after = file
            .metadata()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let current = slot.directory.open_file(&slot.name, false)?;
        if bytes.len() != length
            || !stable(&before, &after)
            || !stable(
                &after,
                &current
                    .metadata()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
            )
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        slot.pending = false;
        self.bytes += length;
        Ok(bytes)
    }

    /// Call only after CEF has physically released every upload/download consumer.
    pub fn finish(&mut self) -> Result<(), BrowserDriverError> {
        self.shutdown_confirmed = true;
        self.cleanup()
    }
    fn cleanup(&mut self) -> Result<(), BrowserDriverError> {
        for slot in &mut self.slots {
            slot.directory.finish()?;
        }
        self.directory.finish()
    }
    fn capacity(&self, bytes: usize) -> Result<(), BrowserDriverError> {
        self.directory.verify_namespace()?;
        if self.slots.len() >= MAX_FILES
            || bytes > MAX_FILE_BYTES
            || self
                .bytes
                .checked_add(bytes)
                .is_none_or(|total| total > MAX_STAGE_BYTES)
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.shutdown_confirmed || self.cleanup().is_err() {
            // The supervisor retains this physical profile allocation on native
            // OutcomeUnknown; never recursively prune an unknown child's tree.
            self.directory.preserve();
            for slot in &mut self.slots {
                slot.directory.preserve();
            }
            eprintln!("browser outcome unknown");
        }
    }
}
fn display_name_leaf(name: &str) -> Result<CString, BrowserDriverError> {
    if name.is_empty()
        || name.len() > 128
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err(BrowserDriverError::Denied);
    }
    CString::new(name).map_err(|_| BrowserDriverError::Denied)
}
fn private_file(metadata: &Metadata, expected: usize) -> Result<(), BrowserDriverError> {
    // SAFETY: geteuid observes process identity and takes no arguments.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() != expected as u64
        || expected > MAX_FILE_BYTES
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}
fn stable(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.uid() == b.uid()
        && a.mode() == b.mode()
        && a.nlink() == b.nlink()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

#[cfg(test)]
#[path = "transfer_stage_unix/tests.rs"]
mod tests;
