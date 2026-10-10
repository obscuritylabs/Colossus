//! Native-only retained Unix directory ownership; no caller pathname authorizes cleanup.
use colossus_native_browser_pki::fingerprint;
use colossus_ports::BrowserDriverError;
use std::{
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd as _, FromRawFd as _, IntoRawFd as _, RawFd},
        unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    },
    path::{Path, PathBuf},
    ptr::NonNull,
};

// libc exposes mode constants as u16 on Darwin and u32 on Linux.
#[allow(clippy::unnecessary_cast)]
const TYPE_MASK: u32 = libc::S_IFMT as u32;
#[allow(clippy::unnecessary_cast)]
const DIRECTORY_TYPE: u32 = libc::S_IFDIR as u32;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
}
impl Identity {
    fn file(file: &File) -> Result<Self, BrowserDriverError> {
        let metadata = file
            .metadata()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
            mode: metadata.mode(),
        })
    }
    fn private_directory(self) -> bool {
        // SAFETY: geteuid reads process identity and takes no arguments.
        self.mode & TYPE_MASK == DIRECTORY_TYPE
            && self.mode & 0o077 == 0
            && self.uid == unsafe { libc::geteuid() }
    }
    fn same_inode(self, other: Self) -> bool {
        self.device == other.device && self.inode == other.inode && self.uid == other.uid
    }
}

/// A fresh private directory held until all native browser processes stop using it.
pub(crate) struct OwnedDirectory {
    path: PathBuf,
    parent: File,
    directory: Option<File>,
    identity: Option<Identity>,
    name: CString,
    retired: Option<CString>,
    removed: bool,
    unknown: bool,
}
impl OwnedDirectory {
    pub(crate) fn create(parent_path: &Path, prefix: &str) -> Result<Self, BrowserDriverError> {
        if !parent_path.is_absolute()
            || parent_path
                .canonicalize()
                .map_err(|_| BrowserDriverError::Denied)?
                != parent_path
        {
            return Err(BrowserDriverError::Denied);
        }
        let expected =
            std::fs::symlink_metadata(parent_path).map_err(|_| BrowserDriverError::Denied)?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent_path)
            .map_err(|_| BrowserDriverError::Denied)?;
        let parent_identity = Identity::file(&parent)?;
        if !parent_identity.private_directory()
            || parent_identity.device != expected.dev()
            || parent_identity.inode != expected.ino()
        {
            return Err(BrowserDriverError::Denied);
        }
        let name = random_name(prefix)?;
        // SAFETY: retained parent fd, generated slash-free terminated basename,
        // and mode0700. Successful mkdirat creates a previously absent entry.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(BrowserDriverError::Denied);
        }
        // A pending owner exists before any fallible inode/descriptor binding.
        let mut owned = Self {
            path: parent_path.join(name.to_str().expect("generated ASCII")),
            parent,
            directory: None,
            identity: None,
            name,
            retired: None,
            removed: false,
            unknown: false,
        };
        let identity = entry(owned.parent.as_raw_fd(), &owned.name)?;
        owned.identity = Some(identity);
        if !identity.private_directory() {
            owned.unknown = true;
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let directory = open_at(
            owned.parent.as_raw_fd(),
            &owned.name,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        if !identity.same_inode(Identity::file(&directory)?) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        owned.directory = Some(directory);
        Ok(owned)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Preserve the physical allocation when a child ownership result is unknown.
    pub(crate) fn preserve(&mut self) {
        self.unknown = true;
    }

    pub(crate) fn verify_namespace(&self) -> Result<(), BrowserDriverError> {
        if self.removed || self.unknown || self.retired.is_some() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let identity = self.identity.ok_or(BrowserDriverError::OutcomeUnknown)?;
        let current = entry(self.parent.as_raw_fd(), &self.name)?;
        let path_identity = std::fs::symlink_metadata(&self.path)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if !identity.same_inode(current)
            || !current.private_directory()
            || path_identity.ino() != identity.inode
            || path_identity.dev() != identity.device
            || self
                .path
                .canonicalize()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
                != self.path
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(())
    }

    pub(crate) fn open_file(&self, name: &CStr, create: bool) -> Result<File, BrowserDriverError> {
        self.verify_namespace()?;
        basename(name)?;
        let directory = self
            .directory
            .as_ref()
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        let flags = if create {
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
        } else {
            libc::O_RDONLY
        };
        open_at(directory.as_raw_fd(), name, flags, 0o600)
    }

    /// Retire the exact owned inode after native shutdown; unknown replacements survive.
    pub(crate) fn finish(&mut self) -> Result<(), BrowserDriverError> {
        if self.unknown {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        if self.removed {
            return self
                .parent
                .sync_all()
                .map_err(|_| BrowserDriverError::OutcomeUnknown);
        }
        let Some(identity) = self.identity else {
            self.unknown = true;
            return Err(BrowserDriverError::OutcomeUnknown);
        };
        if self.retired.is_none() {
            let retired = random_name(".retired-browser-")?;
            if retire(self.parent.as_raw_fd(), &self.name, &retired).is_err() {
                self.unknown = true;
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            self.retired = Some(retired);
        }
        let retired = self.retired.as_ref().expect("retired basename retained");
        let current = entry(self.parent.as_raw_fd(), retired)?;
        if !identity.same_inode(current) || !current.private_directory() {
            self.unknown = true;
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        if let Some(directory) = &self.directory {
            let mut budget = 65_536;
            empty(directory, 0, &mut budget)?;
        }
        // Recheck after descriptor-relative recursion; never unlink an unrelated tree.
        if !identity.same_inode(entry(self.parent.as_raw_fd(), retired)?) {
            self.unknown = true;
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        // SAFETY: retained parent fd and verified exact generated empty directory.
        if unsafe {
            libc::unlinkat(
                self.parent.as_raw_fd(),
                retired.as_ptr(),
                libc::AT_REMOVEDIR,
            )
        } != 0
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        self.removed = true;
        self.parent
            .sync_all()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)
    }
}
impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        if self.finish().is_err() {
            eprintln!("browser outcome unknown");
        }
    }
}

pub(crate) fn random_name(prefix: &str) -> Result<CString, BrowserDriverError> {
    if prefix.is_empty()
        || prefix.len() > 64
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    {
        return Err(BrowserDriverError::Denied);
    }
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|_| BrowserDriverError::Unavailable)?;
    CString::new(format!("{prefix}{}", fingerprint(&nonce))).map_err(|_| BrowserDriverError::Denied)
}
fn basename(name: &CStr) -> Result<(), BrowserDriverError> {
    let bytes = name.to_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}
fn open_at(
    parent: RawFd,
    name: &CStr,
    flags: i32,
    mode: libc::mode_t,
) -> Result<File, BrowserDriverError> {
    // Darwin mode_t is u16; C variadic arguments require integer promotion.
    #[allow(clippy::unnecessary_cast)]
    let mode = mode as libc::c_uint;
    // SAFETY: caller owns parent directory and name is terminated. NOFOLLOW and
    // CLOEXEC prevent indirection and inherited native private-file descriptors.
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    // SAFETY: openat returned a new owned descriptor, transferred exactly once.
    Ok(unsafe { File::from_raw_fd(fd) })
}
// Darwin and Linux stat integer widths differ; casts normalize this private ABI.
#[allow(clippy::unnecessary_cast)]
fn entry(parent: RawFd, name: &CStr) -> Result<Identity, BrowserDriverError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: valid parent/name, writable stat storage; never follow a symlink.
    if unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    // SAFETY: successful fstatat initialized all stat fields.
    let stat = unsafe { stat.assume_init() };
    Ok(Identity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        uid: stat.st_uid,
        mode: stat.st_mode as u32,
    })
}
fn retire(parent: RawFd, from: &CStr, to: &CStr) -> Result<(), BrowserDriverError> {
    #[cfg(target_os = "linux")]
    // SAFETY: retained directory and generated basenames; NOREPLACE protects destination.
    let status = unsafe {
        libc::renameat2(
            parent,
            from.as_ptr(),
            parent,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    // SAFETY: retained directory and generated basenames; EXCL protects destination.
    let status = unsafe {
        libc::renameatx_np(
            parent,
            from.as_ptr(),
            parent,
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if status != 0 {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    Ok(())
}

struct Entries(NonNull<libc::DIR>);
impl Drop for Entries {
    fn drop(&mut self) {
        // SAFETY: DIR is uniquely held here and closed exactly once.
        unsafe { libc::closedir(self.0.as_ptr()) };
    }
}
fn names(directory: &File, budget: &mut usize) -> Result<Vec<CString>, BrowserDriverError> {
    let fd = directory
        .try_clone()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
        .into_raw_fd();
    // SAFETY: transfer the new duplicated directory descriptor to fdopendir.
    let pointer = unsafe { libc::fdopendir(fd) };
    let Some(pointer) = NonNull::new(pointer) else {
        // SAFETY: failed fdopendir did not consume the newly owned descriptor.
        unsafe { libc::close(fd) };
        return Err(BrowserDriverError::OutcomeUnknown);
    };
    let entries = Entries(pointer);
    // SAFETY: unique live DIR, reset for idempotent cleanup retries.
    unsafe { libc::rewinddir(entries.0.as_ptr()) };
    let mut result = Vec::new();
    loop {
        #[cfg(target_os = "linux")]
        // SAFETY: libc returns this thread's live errno pointer.
        let error = unsafe { libc::__errno_location() };
        #[cfg(target_os = "macos")]
        // SAFETY: libc returns this thread's live errno pointer.
        let error = unsafe { libc::__error() };
        // SAFETY: same thread's writable errno pointer, reset before readdir.
        unsafe { *error = 0 };
        // SAFETY: unique live DIR; returned record lives until next readdir.
        let next = unsafe { libc::readdir(entries.0.as_ptr()) };
        if next.is_null() {
            // SAFETY: read this thread's live errno pointer.
            if unsafe { *error } != 0 {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            return Ok(result);
        }
        // SAFETY: POSIX readdir returns a terminated name within its live record.
        let name = unsafe { CStr::from_ptr((*next).d_name.as_ptr()) };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        basename(name)?;
        if *budget == 0 {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        *budget -= 1;
        result.push(name.to_owned());
    }
}
fn empty(directory: &File, depth: usize, budget: &mut usize) -> Result<(), BrowserDriverError> {
    if depth >= 64 {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    for name in names(directory, budget)? {
        let metadata = entry(directory.as_raw_fd(), &name)?;
        let is_directory = metadata.mode & TYPE_MASK == DIRECTORY_TYPE;
        if is_directory {
            if metadata.device != Identity::file(directory)?.device {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            let child = open_at(
                directory.as_raw_fd(),
                &name,
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )?;
            if !metadata.same_inode(Identity::file(&child)?) {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            empty(&child, depth + 1, budget)?;
            if !metadata.same_inode(entry(directory.as_raw_fd(), &name)?) {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        // SAFETY: retained owned tree fd and checked basename. Directory recursion
        // opens NOFOLLOW; unlinking other entries never traverses their targets.
        if unsafe {
            libc::unlinkat(
                directory.as_raw_fd(),
                name.as_ptr(),
                if is_directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } != 0
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
    }
    directory
        .sync_all()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)
}

#[cfg(test)]
mod tests;
