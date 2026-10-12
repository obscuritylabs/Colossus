//! Exact-package private transfer staging; paths never leave the native host.
use colossus_ports::BrowserDriverError;
use colossus_windows_native::{BoundPath, PrivateDirectoryCreation};
use std::{
    fs::{File, OpenOptions},
    io::{Read as _, Write as _},
    os::windows::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};
use zeroize::Zeroizing;

const MAX_BYTES: usize = 4 * 1024 * 1024;
pub struct OwnedFile {
    path: PathBuf,
    _file: File,
}
impl OwnedFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
}
pub struct Reservation {
    stage: String,
    slot: usize,
    path: PathBuf,
}
impl Reservation {
    pub fn path(&self) -> &Path {
        &self.path
    }
}
struct Directory {
    creation: PrivateDirectoryCreation,
    bound: Option<BoundPath>,
    user_development: bool,
}
struct Slot {
    directory: Directory,
    name: String,
    files: Vec<BoundPath>,
    held: Vec<File>,
    pending: bool,
}
pub struct Stage {
    directory: Directory,
    nonce: String,
    slots: Vec<Slot>,
    bytes: usize,
}

impl Directory {
    fn create(
        parent: &BoundPath,
        prefix: &str,
        user_development: bool,
    ) -> Result<Self, BrowserDriverError> {
        let basename = random(prefix)?;
        let creation = if user_development {
            PrivateDirectoryCreation::create(&parent.canonical_path().join(basename))
        } else {
            PrivateDirectoryCreation::create_in_appcontainer_profile(parent, &basename)
        }
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        // Preserve the allocation owner even when metadata binding fails.
        let mut directory = Self {
            creation,
            bound: None,
            user_development,
        };
        directory.bound = Some(
            directory
                .creation
                .bind()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
        );
        Ok(directory)
    }
    fn bound(&self) -> Result<&BoundPath, BrowserDriverError> {
        let bound = self
            .bound
            .as_ref()
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        bound
            .revalidate()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        Ok(bound)
    }
    fn path(&self) -> &Path {
        self.creation.path()
    }
    fn open_file(&self, path: &Path) -> Result<BoundPath, BrowserDriverError> {
        self.bound()?;
        if path.parent() != Some(self.path()) {
            return Err(BrowserDriverError::Denied);
        }
        let bound = if self.user_development {
            let bound =
                BoundPath::open_file(path).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            bound
                .validate_private_owner_dacl()
                .map_err(|_| BrowserDriverError::Denied)?;
            bound
        } else {
            self.bound()?
                .open_appcontainer_file_beneath(path)
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        };
        Ok(bound)
    }
    fn finish(&mut self) -> Result<(), BrowserDriverError> {
        self.bound = None;
        self.creation
            .remove_empty()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)
    }
}
impl Stage {
    pub fn create(profile: &BoundPath) -> Result<Self, BrowserDriverError> {
        profile
            .validate_private_appcontainer_dacl()
            .map_err(|_| BrowserDriverError::Denied)?;
        Self::create_inner(profile, false)
    }
    #[cfg(debug_assertions)]
    pub fn create_development(profile: &BoundPath) -> Result<Self, BrowserDriverError> {
        profile
            .validate_private_owner_dacl()
            .map_err(|_| BrowserDriverError::Denied)?;
        Self::create_inner(profile, true)
    }
    fn create_inner(
        profile: &BoundPath,
        user_development: bool,
    ) -> Result<Self, BrowserDriverError> {
        Ok(Self {
            directory: Directory::create(profile, "transfer-", user_development)?,
            nonce: random("stage-")?,
            slots: Vec::new(),
            bytes: 0,
        })
    }
    fn capacity(&self, bytes: usize) -> Result<(), BrowserDriverError> {
        self.directory.bound()?;
        if self.slots.len() >= 8
            || bytes > MAX_BYTES
            || self
                .bytes
                .checked_add(bytes)
                .is_none_or(|sum| sum > 8 * MAX_BYTES)
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        Ok(())
    }
    pub fn upload(&mut self, bytes: &[u8], name: &str) -> Result<OwnedFile, BrowserDriverError> {
        self.capacity(bytes.len())?;
        if name.len() > 64
            || !name.starts_with("upload.")
            || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.')
        {
            return Err(BrowserDriverError::Denied);
        }
        let directory = Directory::create(
            self.directory.bound()?,
            "upload-",
            self.directory.user_development,
        )?;
        self.slots.push(Slot {
            directory,
            name: name.to_owned(),
            files: Vec::new(),
            held: Vec::new(),
            pending: false,
        });
        let slot = self.slots.last_mut().expect("inserted upload");
        let path = slot.directory.path().join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let bound = slot.directory.open_file(&path)?;
        if bound.identity()
            != colossus_windows_native::FileIdentity::of(&file)
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            || file
                .metadata()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
                .len()
                != bytes.len() as u64
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        // Chromium opens uploads without sharing existing writers. Keeping a
        // cloned write handle made Windows reject the real network-service read.
        // Close the writer, then retain only read handles that deny new writers.
        drop(file);
        let mut frozen = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if colossus_windows_native::FileIdentity::of(&frozen)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            != bound.identity()
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let mut observed = Zeroizing::new(Vec::with_capacity(bytes.len()));
        std::io::Read::by_ref(&mut frozen)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut observed)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if observed.as_slice() != bytes {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        slot.held.push(
            frozen
                .try_clone()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
        );
        slot.files.push(bound);
        self.bytes += bytes.len();
        Ok(OwnedFile {
            path,
            _file: frozen,
        })
    }
    pub fn reserve_download(&mut self) -> Result<Reservation, BrowserDriverError> {
        self.capacity(0)?;
        let directory = Directory::create(
            self.directory.bound()?,
            "download-",
            self.directory.user_development,
        )?;
        let path = directory.path().join("download");
        let index = self.slots.len();
        self.slots.push(Slot {
            directory,
            name: "download".to_owned(),
            files: Vec::new(),
            held: Vec::new(),
            pending: true,
        });
        Ok(Reservation {
            stage: self.nonce.clone(),
            slot: index,
            path,
        })
    }
    pub fn completed(
        &mut self,
        reservation: Reservation,
        size: u32,
    ) -> Result<Zeroizing<Vec<u8>>, BrowserDriverError> {
        let length = size as usize;
        if reservation.stage != self.nonce
            || length > MAX_BYTES
            || self
                .bytes
                .checked_add(length)
                .is_none_or(|sum| sum > 8 * MAX_BYTES)
        {
            return Err(BrowserDriverError::Denied);
        }
        self.directory.bound()?;
        let slot = self
            .slots
            .get_mut(reservation.slot)
            .ok_or(BrowserDriverError::Denied)?;
        slot.directory.bound()?;
        if !slot.pending || reservation.path != slot.directory.path().join("download") {
            return Err(BrowserDriverError::Denied);
        }
        let bound = slot.directory.open_file(&reservation.path)?;
        // A retained no-write-sharing reader freezes bytes after CEF's actual
        // completion; the separate binding proves exact package ACL/identity.
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&reservation.path)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if bound.identity()
            != colossus_windows_native::FileIdentity::of(&file)
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let before = file
            .metadata()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if !before.is_file() || before.len() != length as u64 {
            return Err(BrowserDriverError::Denied);
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(length));
        std::io::Read::by_ref(&mut file)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let after = file
            .metadata()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        bound
            .revalidate()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if bytes.len() != length
            || before.len() != after.len()
            || before.last_write_time() != after.last_write_time()
            || before.file_attributes() != after.file_attributes()
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        slot.files.push(bound);
        slot.held.push(file);
        slot.pending = false;
        self.bytes += length;
        Ok(bytes)
    }
    /// Called only after CefShutdown; any unknown file remains owned by the supervisor's profile.
    pub fn finish(&mut self) -> Result<(), BrowserDriverError> {
        if self.directory.bound.is_some() {
            self.directory.bound()?;
        }
        for slot in &mut self.slots {
            if slot.directory.bound.is_none() {
                slot.directory.finish()?;
                continue;
            }
            slot.directory.bound()?;
            // CEF has released every consumer. Retire no-write-sharing byte
            // leases before exact DELETE handles are acquired for cleanup.
            slot.held.clear();
            for row in std::fs::read_dir(slot.directory.path())
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            {
                let row = row.map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                if row.file_name() != std::ffi::OsStr::new(&slot.name)
                    && row.file_name() != "download.crdownload"
                {
                    return Err(BrowserDriverError::OutcomeUnknown);
                }
                let file = slot.directory.open_file(&row.path())?;
                if slot.directory.user_development {
                    file.revalidate()
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                    std::fs::remove_file(file.canonical_path())
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                } else {
                    file.remove_appcontainer_file()
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                }
            }
            slot.files.clear();
            slot.directory.finish()?;
        }
        self.directory.finish()
    }
}
fn random(prefix: &str) -> Result<String, BrowserDriverError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| BrowserDriverError::Failed)?;
    Ok(format!(
        "{prefix}{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
