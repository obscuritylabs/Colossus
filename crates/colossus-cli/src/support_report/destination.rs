use super::{RunControl, SaveError, check_cancelled};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use rustix::{
    fd::OwnedFd,
    fs::{self, AtFlags, FileType, Mode, OFlags},
    io::Errno,
};
#[cfg(not(unix))]
use std::{fs, io};

pub(super) struct Destination {
    parent: PathBuf,
    name: OsString,
    #[cfg(unix)]
    directory: OwnedFd,
    #[cfg(windows)]
    binding: Option<colossus_windows_native::BoundPath>,
}

impl Destination {
    pub(super) fn open(output: &Path) -> Result<Self, SaveError> {
        let name = output_name(output)?.to_owned();
        let parent = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .map_err(|_| SaveError::Destination)?;
        if !local_path_syntax(&parent) || !parent.is_dir() {
            return Err(SaveError::Destination);
        }
        #[cfg(unix)]
        let directory = fs::open(
            &parent,
            directory_access() | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| SaveError::Destination)?;
        #[cfg(windows)]
        let binding = if parent.parent().is_some() {
            Some(
                colossus_windows_native::BoundPath::open_stable_directory(&parent)
                    .map_err(|_| SaveError::Destination)?,
            )
        } else {
            // A local drive root cannot be renamed as a directory entry.
            None
        };
        Ok(Self {
            parent,
            name,
            #[cfg(unix)]
            directory,
            #[cfg(windows)]
            binding,
        })
    }

    fn revalidate_parent(&self) -> Result<(), SaveError> {
        #[cfg(unix)]
        {
            let retained = fs::fstat(&self.directory).map_err(|_| SaveError::Destination)?;
            let named = fs::statat(fs::CWD, &self.parent, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| SaveError::Destination)?;
            if (retained.st_dev, retained.st_ino) != (named.st_dev, named.st_ino) {
                return Err(SaveError::Destination);
            }
        }
        #[cfg(windows)]
        if let Some(binding) = &self.binding {
            binding.revalidate().map_err(|_| SaveError::Destination)?;
        }
        Ok(())
    }

    pub(super) fn check_leaf(&self, replace: bool) -> Result<(), SaveError> {
        #[cfg(unix)]
        let regular_file = match fs::statat(&self.directory, &self.name, AtFlags::SYMLINK_NOFOLLOW)
        {
            Ok(metadata) => FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile,
            Err(Errno::NOENT) => return Ok(()),
            Err(_) => return Err(SaveError::Destination),
        };
        #[cfg(not(unix))]
        let regular_file = match fs::symlink_metadata(self.parent.join(&self.name)) {
            Ok(metadata) => metadata.is_file(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(SaveError::Destination),
        };
        if !replace {
            return Err(SaveError::Exists);
        }
        if !regular_file {
            return Err(SaveError::Destination);
        }
        Ok(())
    }

    pub(super) fn stage(&self) -> Result<StagedFile<'_>, SaveError> {
        self.revalidate_parent()?;
        #[cfg(unix)]
        {
            let name = format!(".colossus-support-{}.tmp", uuid::Uuid::now_v7());
            let file = fs::openat(
                &self.directory,
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|_| SaveError::Save)?;
            Ok(StagedFile {
                destination: self,
                name,
                file: file.into(),
                clean_up: true,
            })
        }
        #[cfg(not(unix))]
        {
            let temporary =
                tempfile::NamedTempFile::new_in(&self.parent).map_err(|_| SaveError::Save)?;
            Ok(StagedFile {
                destination: self,
                temporary,
            })
        }
    }
}

pub(super) struct StagedFile<'a> {
    destination: &'a Destination,
    #[cfg(unix)]
    name: String,
    #[cfg(unix)]
    file: File,
    #[cfg(unix)]
    clean_up: bool,
    #[cfg(not(unix))]
    temporary: tempfile::NamedTempFile,
}

impl StagedFile<'_> {
    pub(super) fn file_mut(&mut self) -> &mut File {
        #[cfg(unix)]
        {
            &mut self.file
        }
        #[cfg(not(unix))]
        {
            self.temporary.as_file_mut()
        }
    }

    pub(super) fn publish(self, replace: bool, control: &RunControl) -> Result<(), SaveError> {
        self.destination.revalidate_parent()?;
        self.destination.check_leaf(replace)?;
        check_cancelled(control)?;
        #[cfg(unix)]
        {
            self.publish_relative(replace)
        }
        #[cfg(not(unix))]
        {
            let target = self.destination.parent.join(&self.destination.name);
            let result = if replace {
                self.temporary.persist(target)
            } else {
                self.temporary.persist_noclobber(target)
            };
            result
                .map(|_| ())
                .map_err(|error| match error.error.kind() {
                    io::ErrorKind::AlreadyExists if !replace => SaveError::Exists,
                    _ => SaveError::Save,
                })
        }
    }
}

#[cfg(unix)]
impl StagedFile<'_> {
    fn publish_relative(mut self, replace: bool) -> Result<(), SaveError> {
        if !self.stage_name_matches() {
            return Err(SaveError::Save);
        }
        let parent = &self.destination.directory;
        let name = self.name.as_str();
        let target = &self.destination.name;
        let renamed = if replace {
            fs::renameat(parent, name, parent, target).map(|()| true)
        } else {
            publish_new_file(parent, name, target)
        };
        match renamed {
            Ok(consumed_stage) => {
                self.clean_up = !consumed_stage;
                Ok(())
            }
            Err(Errno::EXIST) if !replace => Err(SaveError::Exists),
            Err(_) => Err(SaveError::Save),
        }
    }

    fn stage_name_matches(&self) -> bool {
        let Ok(retained) = fs::fstat(&self.file) else {
            return false;
        };
        fs::statat(
            &self.destination.directory,
            self.name.as_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .is_ok_and(|named| (retained.st_dev, retained.st_ino) == (named.st_dev, named.st_ino))
    }
}

#[cfg(unix)]
fn publish_new_file(parent: &OwnedFd, name: &str, target: &OsStr) -> Result<bool, Errno> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    match fs::renameat_with(parent, name, parent, target, fs::RenameFlags::NOREPLACE) {
        Ok(()) => return Ok(true),
        Err(Errno::NOSYS | Errno::INVAL | Errno::NOTSUP) => {}
        Err(error) => return Err(error),
    }
    fs::linkat(parent, name, parent, target, AtFlags::empty()).map(|()| false)
}

#[cfg(target_os = "linux")]
fn directory_access() -> OFlags {
    OFlags::PATH
}

#[cfg(all(unix, not(target_os = "linux")))]
fn directory_access() -> OFlags {
    OFlags::RDONLY
}

#[cfg(unix)]
impl Drop for StagedFile<'_> {
    fn drop(&mut self) {
        if self.clean_up && self.stage_name_matches() {
            let _ = fs::unlinkat(
                &self.destination.directory,
                self.name.as_str(),
                AtFlags::empty(),
            );
        }
    }
}

pub(super) fn output_name(path: &Path) -> Result<&OsStr, SaveError> {
    let trailing = path
        .as_os_str()
        .as_encoded_bytes()
        .rsplit(|byte| *byte == b'/' || (cfg!(windows) && *byte == b'\\'))
        .next()
        .ok_or(SaveError::Destination)?;
    if matches!(trailing, b"" | b"." | b".." | b"-")
        || (cfg!(windows) && matches!(trailing.last(), Some(b'.' | b' ')))
        || !local_path_syntax(path)
    {
        return Err(SaveError::Destination);
    }
    #[cfg(windows)]
    if reserved_windows_leaf(trailing) {
        return Err(SaveError::Destination);
    }
    path.file_name().ok_or(SaveError::Destination)
}

#[cfg(windows)]
fn reserved_windows_leaf(name: &[u8]) -> bool {
    let stem = name.split(|byte| *byte == b'.').next().unwrap_or_default();
    [
        b"CON".as_slice(),
        b"PRN",
        b"AUX",
        b"NUL",
        b"CONIN$",
        b"CONOUT$",
    ]
    .iter()
    .any(|reserved| stem.eq_ignore_ascii_case(reserved))
        || stem.get(..3).is_some_and(|prefix| {
            (prefix.eq_ignore_ascii_case(b"COM") || prefix.eq_ignore_ascii_case(b"LPT"))
                && matches!(
                    &stem[3..],
                    [b'1'..=b'9'] | b"\xc2\xb9" | b"\xc2\xb2" | b"\xc2\xb3"
                )
        })
}

#[cfg(windows)]
pub(super) fn local_path_syntax(path: &Path) -> bool {
    use std::path::{Component, Prefix};

    path.components().all(|component| match component {
        Component::Prefix(prefix) => {
            path.is_absolute() && matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        }
        Component::Normal(name) => !name.as_encoded_bytes().contains(&b':'),
        _ => true,
    })
}

#[cfg(not(windows))]
fn local_path_syntax(_path: &Path) -> bool {
    true
}
