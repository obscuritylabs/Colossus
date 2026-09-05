//! Read-only, handle-bound access to portable content. Never follows plugin links.

use super::*;

pub(crate) struct ReadRoot {
    path: PathBuf,
    #[cfg(unix)]
    directory: File,
    #[cfg(windows)]
    directory: colossus_windows_native::BoundPath,
}

pub(crate) struct ReadEntry {
    pub path: PathBuf,
    pub directory: bool,
    pub size: u64,
}

impl ReadRoot {
    pub fn bind(path: &Path) -> Result<Self, StoreError> {
        let before = fs::symlink_metadata(path).map_err(adapter)?;
        if before.file_type().is_symlink() || !before.is_dir() {
            return Err(adapter("plugin root is not a real directory"));
        }
        let path = fs::canonicalize(path).map_err(adapter)?;
        #[cfg(unix)]
        let directory = {
            use std::os::unix::fs::MetadataExt as _;
            let directory = File::from(
                rustix::fs::open(
                    &path,
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::DIRECTORY
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .map_err(adapter)?,
            );
            let opened = directory.metadata().map_err(adapter)?;
            if (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
                return Err(adapter("plugin root changed while opening"));
            }
            directory
        };
        #[cfg(windows)]
        let directory =
            colossus_windows_native::BoundPath::open_directory(&path).map_err(adapter)?;
        let root = Self { path, directory };
        root.revalidate()?;
        Ok(root)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn revalidate(&self) -> Result<(), StoreError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let current = fs::symlink_metadata(&self.path).map_err(adapter)?;
            let opened = self.directory.metadata().map_err(adapter)?;
            if !current.is_dir()
                || current.file_type().is_symlink()
                || (current.dev(), current.ino()) != (opened.dev(), opened.ino())
            {
                return Err(adapter("plugin root changed during access"));
            }
        }
        #[cfg(windows)]
        self.directory.revalidate().map_err(adapter)?;
        Ok(())
    }

    #[cfg(unix)]
    fn open(&self, relative: &Path, directory: bool) -> Result<File, StoreError> {
        self.revalidate()?;
        let mut current = self.directory.try_clone().map_err(adapter)?;
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(adapter("plugin path is not a normalized relative path"));
            };
            let mut flags = rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NONBLOCK;
            if components.peek().is_some() || directory {
                flags |= rustix::fs::OFlags::DIRECTORY;
            }
            current = File::from(
                rustix::fs::openat(&current, name, flags, rustix::fs::Mode::empty())
                    .map_err(adapter)?,
            );
        }
        self.revalidate()?;
        Ok(current)
    }

    pub fn open_file(&self, relative: &Path, maximum: u64) -> Result<File, StoreError> {
        if relative.as_os_str().is_empty() {
            return Err(adapter("plugin file path is empty"));
        }
        posix_path(relative)?;
        #[cfg(unix)]
        let file = self.open(relative, false)?;
        #[cfg(windows)]
        let binding = colossus_windows_native::BoundPath::open_file(&self.path.join(relative))
            .map_err(adapter)?;
        #[cfg(windows)]
        let file = binding.try_clone_file().map_err(adapter)?;
        let metadata = file.metadata().map_err(adapter)?;
        // Verified blob caches may retain another hard link to this same regular
        // object. Archive link entries are rejected independently at extraction.
        if !metadata.is_file() || metadata.len() > maximum {
            return Err(adapter(format!(
                "plugin file is linked, special, or exceeds {maximum} bytes"
            )));
        }
        #[cfg(windows)]
        binding.revalidate().map_err(adapter)?;
        self.revalidate()?;
        Ok(file)
    }

    pub fn read(&self, relative: &Path, maximum: u64) -> Result<Vec<u8>, StoreError> {
        let file = self.open_file(relative, maximum)?;
        let mut bytes = Vec::new();
        file.take(maximum.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(adapter)?;
        if bytes.len() as u64 > maximum {
            return Err(adapter("plugin file exceeded its read limit"));
        }
        self.revalidate()?;
        Ok(bytes)
    }

    pub fn entries(&self, relative: &Path) -> Result<Vec<ReadEntry>, StoreError> {
        self.revalidate()?;
        #[cfg(unix)]
        let entries = self.unix_entries(relative)?;
        #[cfg(windows)]
        let entries = self.windows_entries(relative)?;
        self.revalidate()?;
        Ok(entries)
    }

    #[cfg(unix)]
    fn unix_entries(&self, relative: &Path) -> Result<Vec<ReadEntry>, StoreError> {
        use std::os::unix::ffi::OsStrExt as _;
        let directory = self.open(relative, true)?;
        let mut entries = Vec::new();
        for entry in rustix::fs::Dir::read_from(&directory).map_err(adapter)? {
            let entry = entry.map_err(adapter)?;
            let name = entry.file_name();
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            if entries.len() >= MAX_FILES {
                return Err(adapter("plugin directory entry limit exceeded"));
            }
            let stat = rustix::fs::statat(&directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                .map_err(adapter)?;
            let kind = rustix::fs::FileType::from_raw_mode(stat.st_mode);
            if !matches!(
                kind,
                rustix::fs::FileType::Directory | rustix::fs::FileType::RegularFile
            ) {
                return Err(adapter("plugin contains a link or special file"));
            }
            entries.push(ReadEntry {
                path: relative.join(std::ffi::OsStr::from_bytes(name.to_bytes())),
                directory: kind == rustix::fs::FileType::Directory,
                size: u64::try_from(stat.st_size).map_err(adapter)?,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    #[cfg(windows)]
    fn windows_entries(&self, relative: &Path) -> Result<Vec<ReadEntry>, StoreError> {
        if !relative.as_os_str().is_empty() {
            posix_path(relative)?;
        }
        let directory =
            colossus_windows_native::BoundPath::open_directory(&self.path.join(relative))
                .map_err(adapter)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(directory.canonical_path()).map_err(adapter)? {
            let entry = entry.map_err(adapter)?;
            if entries.len() >= MAX_FILES {
                return Err(adapter("plugin directory entry limit exceeded"));
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(adapter)?;
            if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
                return Err(adapter("plugin contains a link or special file"));
            }
            entries.push(ReadEntry {
                path: relative.join(entry.file_name()),
                directory: metadata.is_dir(),
                size: metadata.len(),
            });
        }
        directory.revalidate().map_err(adapter)?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }
}

pub(crate) fn read_contained(
    root: &Path,
    relative: &Path,
    maximum: u64,
) -> Result<Vec<u8>, StoreError> {
    ReadRoot::bind(root)?.read(relative, maximum)
}
