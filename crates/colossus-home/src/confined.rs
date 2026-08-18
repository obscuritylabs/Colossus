use crate::{HomeError, home::ensure_private_directory};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[cfg(windows)]
const WINDOWS_SHARING_VIOLATION: i32 = 32;
#[cfg(windows)]
const WINDOWS_RACE_RETRIES: usize = 100;
#[cfg(windows)]
const WINDOWS_RACE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

/// A retained owner-private directory used as the authority for derived state paths.
///
/// Relative components are opened one at a time without following links. On Unix the
/// retained directory descriptor is also used for descriptor-relative file creation;
/// on Windows `BoundPath` retains every opened ancestor and rejects reparse points.
#[derive(Clone)]
pub struct ConfinedRoot {
    path: PathBuf,
    #[cfg(unix)]
    directory: Arc<File>,
    #[cfg(windows)]
    binding: Arc<colossus_windows_native::BoundPath>,
}

impl ConfinedRoot {
    /// Bind one existing absolute owner-private directory without following links.
    pub fn bind(path: impl AsRef<Path>) -> Result<Self, HomeError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(HomeError::UnsafeConfinedPath(path.to_owned()));
        }
        ensure_private_directory(path)?;
        #[cfg(windows)]
        return bind_platform_root(path.to_owned());

        #[cfg(not(windows))]
        let canonical = fs::canonicalize(path).map_err(|error| HomeError::io(path, error))?;
        #[cfg(not(windows))]
        if canonical != path {
            return Err(HomeError::UnsafeConfinedPath(path.to_owned()));
        }
        #[cfg(not(windows))]
        bind_platform_root(canonical)
    }

    /// Exact absolute path bound by this root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolve a confined relative file path, creating only missing private parents.
    ///
    /// An existing leaf is opened without following links and must be an owner-private,
    /// single-link regular file. The leaf itself is not created by this operation.
    pub fn prepare_file(&self, relative: &Path) -> Result<PathBuf, HomeError> {
        let (parents, leaf) = relative_file_parts(relative)?;
        prepare_file_platform(self, &parents, &leaf)?;
        self.revalidate()?;
        Ok(self.path.join(relative))
    }

    /// Securely open or create a confined regular file for read/write access.
    pub fn open_file(&self, relative: &Path) -> Result<ConfinedFile, HomeError> {
        let (parents, leaf) = relative_file_parts(relative)?;
        let (file, created) = open_file_platform(self, &parents, &leaf)?;
        self.revalidate()?;
        Ok(ConfinedFile {
            path: self.path.join(relative),
            file,
            created,
        })
    }

    /// Open one existing confined regular file without creating a missing leaf.
    pub fn open_existing_file(&self, relative: &Path) -> Result<ConfinedFile, HomeError> {
        let (parents, leaf) = relative_file_parts(relative)?;
        let file = open_existing_file_platform(self, &parents, &leaf)?;
        self.revalidate()?;
        Ok(ConfinedFile {
            path: self.path.join(relative),
            file,
            created: false,
        })
    }

    /// Ensure and bind a confined owner-private directory path.
    pub fn prepare_directory(&self, relative: &Path) -> Result<PathBuf, HomeError> {
        let components = relative_components(relative)?;
        if components.is_empty() {
            return Err(HomeError::UnsafeConfinedPath(relative.to_owned()));
        }
        prepare_directory_platform(self, &components)?;
        self.revalidate()?;
        Ok(self.path.join(relative))
    }

    /// Revalidate an absolute file path already derived from this root.
    pub fn revalidate_file(&self, path: &Path) -> Result<(), HomeError> {
        let relative = self.relative(path)?;
        self.prepare_file(relative).map(|_| ())
    }

    /// Revalidate an absolute directory path already derived from this root.
    pub fn revalidate_directory(&self, path: &Path) -> Result<(), HomeError> {
        let relative = self.relative(path)?;
        let components = relative_components(relative)?;
        if components.is_empty() {
            return Err(HomeError::UnsafeConfinedPath(path.to_owned()));
        }
        validate_directory_platform(self, &components)?;
        self.revalidate()
    }

    /// Return the confined relative form of one absolute derived path.
    pub fn relative<'a>(&self, path: &'a Path) -> Result<&'a Path, HomeError> {
        let relative = path
            .strip_prefix(&self.path)
            .map_err(|_| HomeError::UnsafeConfinedPath(path.to_owned()))?;
        if relative.as_os_str().is_empty() || relative_components(relative).is_err() {
            return Err(HomeError::UnsafeConfinedPath(path.to_owned()));
        }
        Ok(relative)
    }

    /// Revalidate that the retained directory is still named by the original path.
    ///
    /// Call this after descriptor-relative reads when a namespace replacement must
    /// invalidate the whole operation rather than merely preserve access to the
    /// displaced directory object.
    pub fn revalidate(&self) -> Result<(), HomeError> {
        revalidate_platform_root(self)
    }
}

impl std::fmt::Debug for ConfinedRoot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfinedRoot")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ConfinedRoot {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for ConfinedRoot {}

/// One descriptor-relative file open beneath a [`ConfinedRoot`].
pub struct ConfinedFile {
    path: PathBuf,
    file: File,
    created: bool,
}

impl ConfinedFile {
    /// Diagnostic path associated with the retained file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether this call created the previously absent file.
    pub const fn was_created(&self) -> bool {
        self.created
    }

    /// Consume the binding and return its read/write file descriptor.
    pub fn into_file(self) -> File {
        self.file
    }

    /// Consume the binding and return the file plus its creation status.
    pub fn into_parts(self) -> (File, bool) {
        (self.file, self.created)
    }
}

impl std::fmt::Debug for ConfinedFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfinedFile")
            .field("path", &self.path)
            .field("created", &self.created)
            .finish_non_exhaustive()
    }
}

fn relative_components(path: &Path) -> Result<Vec<OsString>, HomeError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(HomeError::UnsafeConfinedPath(path.to_owned()));
    }
    path.components()
        .map(|component| match component {
            Component::Normal(component) => Ok(component.to_owned()),
            _ => Err(HomeError::UnsafeConfinedPath(path.to_owned())),
        })
        .collect()
}

fn relative_file_parts(path: &Path) -> Result<(Vec<OsString>, OsString), HomeError> {
    let mut components = relative_components(path)?;
    let leaf = components
        .pop()
        .ok_or_else(|| HomeError::UnsafeConfinedPath(path.to_owned()))?;
    Ok((components, leaf))
}

#[cfg(unix)]
fn bind_platform_root(path: PathBuf) -> Result<ConfinedRoot, HomeError> {
    use std::os::unix::fs::MetadataExt as _;

    let before = fs::symlink_metadata(&path).map_err(|error| HomeError::io(&path, error))?;
    let directory = rustix::fs::open(
        &path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| HomeError::io(&path, error.into()))?;
    let opened = directory
        .metadata()
        .map_err(|error| HomeError::io(&path, error))?;
    let after = fs::symlink_metadata(&path).map_err(|error| HomeError::io(&path, error))?;
    if !private_directory_metadata(&opened)
        || before.file_type().is_symlink()
        || before.dev() != opened.dev()
        || before.ino() != opened.ino()
        || after.file_type().is_symlink()
        || after.dev() != opened.dev()
        || after.ino() != opened.ino()
    {
        return Err(HomeError::UnsafeConfinedPath(path));
    }
    Ok(ConfinedRoot {
        path,
        directory: Arc::new(directory),
    })
}

#[cfg(unix)]
fn revalidate_platform_root(root: &ConfinedRoot) -> Result<(), HomeError> {
    use std::os::unix::fs::MetadataExt as _;

    let current = rustix::fs::open(
        &root.path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| HomeError::io(&root.path, error.into()))?;
    let expected = root
        .directory
        .metadata()
        .map_err(|error| HomeError::io(&root.path, error))?;
    let actual = current
        .metadata()
        .map_err(|error| HomeError::io(&root.path, error))?;
    if !private_directory_metadata(&expected)
        || !private_directory_metadata(&actual)
        || expected.dev() != actual.dev()
        || expected.ino() != actual.ino()
    {
        return Err(HomeError::UnsafeConfinedPath(root.path.clone()));
    }
    Ok(())
}

#[cfg(unix)]
fn private_directory_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    metadata.is_dir()
        && metadata.uid() == rustix::process::geteuid().as_raw()
        && metadata.mode() & 0o077 == 0
}

#[cfg(unix)]
fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    metadata.is_file()
        && metadata.uid() == rustix::process::geteuid().as_raw()
        && metadata.nlink() == 1
}

#[cfg(unix)]
fn open_directory_components(
    root: &ConfinedRoot,
    components: &[OsString],
    create: bool,
) -> Result<File, HomeError> {
    let mut current = root
        .directory
        .try_clone()
        .map_err(|error| HomeError::io(&root.path, error))?;
    let mut display_path = root.path.clone();
    for component in components {
        display_path.push(component);
        let opened = rustix::fs::openat(
            &current,
            component,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        );
        let opened = match opened {
            Ok(opened) => opened,
            Err(error) if create && error == rustix::io::Errno::NOENT => {
                match rustix::fs::mkdirat(
                    &current,
                    component,
                    rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR | rustix::fs::Mode::XUSR,
                ) {
                    Ok(()) => {}
                    Err(error) if error == rustix::io::Errno::EXIST => {}
                    Err(error) => return Err(HomeError::io(&display_path, error.into())),
                }
                rustix::fs::openat(
                    &current,
                    component,
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::DIRECTORY
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .map_err(|error| HomeError::io(&display_path, error.into()))?
            }
            Err(error) => return Err(HomeError::io(&display_path, error.into())),
        };
        let opened = File::from(opened);
        let metadata = opened
            .metadata()
            .map_err(|error| HomeError::io(&display_path, error))?;
        if !private_directory_metadata(&metadata) {
            return Err(HomeError::UnsafeConfinedPath(display_path));
        }
        current = opened;
    }
    Ok(current)
}

#[cfg(unix)]
fn prepare_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<(), HomeError> {
    let parent = open_directory_components(root, parents, true)?;
    let path = root.path.join(PathBuf::from_iter(parents)).join(leaf);
    match rustix::fs::openat(
        &parent,
        leaf,
        rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(file) => {
            let metadata = File::from(file)
                .metadata()
                .map_err(|error| HomeError::io(&path, error))?;
            if !private_file_metadata(&metadata) {
                return Err(HomeError::UnsafeConfinedPath(path));
            }
            Ok(())
        }
        Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
        Err(error) => Err(HomeError::io(&path, error.into())),
    }
}

#[cfg(unix)]
fn open_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<(File, bool), HomeError> {
    let parent = open_directory_components(root, parents, true)?;
    let path = root.path.join(PathBuf::from_iter(parents)).join(leaf);
    let flags = rustix::fs::OFlags::RDWR
        | rustix::fs::OFlags::NOFOLLOW
        | rustix::fs::OFlags::NONBLOCK
        | rustix::fs::OFlags::CLOEXEC;
    let (file, created) = match rustix::fs::openat(
        &parent,
        leaf,
        flags | rustix::fs::OFlags::CREATE | rustix::fs::OFlags::EXCL,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    ) {
        Ok(file) => (file, true),
        Err(error) if error == rustix::io::Errno::EXIST => (
            rustix::fs::openat(&parent, leaf, flags, rustix::fs::Mode::empty())
                .map_err(|error| HomeError::io(&path, error.into()))?,
            false,
        ),
        Err(error) => return Err(HomeError::io(&path, error.into())),
    };
    let file = File::from(file);
    let metadata = file
        .metadata()
        .map_err(|error| HomeError::io(&path, error))?;
    if !private_file_metadata(&metadata) {
        return Err(HomeError::UnsafeConfinedPath(path));
    }
    Ok((file, created))
}

#[cfg(unix)]
fn open_existing_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<File, HomeError> {
    let parent = open_directory_components(root, parents, false)?;
    let path = root.path.join(PathBuf::from_iter(parents)).join(leaf);
    let file = rustix::fs::openat(
        &parent,
        leaf,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| HomeError::io(&path, error.into()))?;
    let metadata = file
        .metadata()
        .map_err(|error| HomeError::io(&path, error))?;
    if !private_file_metadata(&metadata) {
        return Err(HomeError::UnsafeConfinedPath(path));
    }
    Ok(file)
}

#[cfg(unix)]
fn prepare_directory_platform(
    root: &ConfinedRoot,
    components: &[OsString],
) -> Result<(), HomeError> {
    open_directory_components(root, components, true).map(|_| ())
}

#[cfg(unix)]
fn validate_directory_platform(
    root: &ConfinedRoot,
    components: &[OsString],
) -> Result<(), HomeError> {
    open_directory_components(root, components, false).map(|_| ())
}

#[cfg(windows)]
fn bind_platform_root(path: PathBuf) -> Result<ConfinedRoot, HomeError> {
    let binding = colossus_windows_native::BoundPath::open_directory(&path)
        .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
    binding
        .validate_ancestor_namespace_authority()
        .and_then(|()| binding.validate_private_owner_dacl())
        .and_then(|()| binding.revalidate())
        .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
    Ok(ConfinedRoot {
        path,
        binding: Arc::new(binding),
    })
}

#[cfg(windows)]
fn revalidate_platform_root(root: &ConfinedRoot) -> Result<(), HomeError> {
    root.binding
        .validate_ancestor_namespace_authority()
        .and_then(|()| root.binding.validate_private_owner_dacl())
        .and_then(|()| root.binding.revalidate())
        .map_err(|_| HomeError::UnsafeConfinedPath(root.path.clone()))
}

#[cfg(windows)]
fn windows_directory_path(
    root: &ConfinedRoot,
    components: &[OsString],
    create: bool,
) -> Result<PathBuf, HomeError> {
    let mut path = root.path.clone();
    for component in components {
        path.push(component);
        if create {
            match colossus_windows_native::create_private_directory(&path) {
                Ok(()) => {}
                Err(colossus_windows_native::WindowsNativeError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(HomeError::UnsafeConfinedPath(path.clone())),
            }
        }
        let binding = colossus_windows_native::BoundPath::open_directory(&path)
            .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
        binding
            .validate_private_owner_dacl()
            .and_then(|()| binding.revalidate())
            .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
    }
    Ok(path)
}

#[cfg(windows)]
fn prepare_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<(), HomeError> {
    let path = windows_directory_path(root, parents, true)?.join(leaf);
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            let binding = colossus_windows_native::BoundPath::open_file(&path)
                .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
            binding
                .validate_private_owner_dacl()
                .and_then(|()| binding.revalidate())
                .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
            if binding.link_count().ok() != Some(1) {
                return Err(HomeError::UnsafeConfinedPath(path));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(HomeError::io(&path, error)),
    }
}

#[cfg(windows)]
fn open_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<(File, bool), HomeError> {
    let path = windows_directory_path(root, parents, true)?.join(leaf);
    let (binding, created) = open_or_create_windows_private_file(&path)?;
    binding
        .validate_private_owner_dacl()
        .and_then(|()| binding.revalidate())
        .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
    if binding.link_count().ok() != Some(1) {
        return Err(HomeError::UnsafeConfinedPath(path));
    }
    let file = binding
        .try_clone_file()
        .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
    Ok((file, created))
}

#[cfg(windows)]
fn open_or_create_windows_private_file(
    path: &Path,
) -> Result<(colossus_windows_native::BoundPath, bool), HomeError> {
    let mut created = false;
    let mut transient_failures = 0;
    while transient_failures < WINDOWS_RACE_RETRIES {
        match colossus_windows_native::BoundPath::open_file_read_write(path) {
            Ok(binding) => return Ok((binding, created)),
            Err(colossus_windows_native::WindowsNativeError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                match colossus_windows_native::create_private_file(path, &[]) {
                    Ok(()) => {
                        created = true;
                        continue;
                    }
                    Err(colossus_windows_native::WindowsNativeError::Io { source, .. })
                        if source.kind() == std::io::ErrorKind::AlreadyExists
                            || source.raw_os_error() == Some(WINDOWS_SHARING_VIOLATION) => {}
                    Err(_) => return Err(HomeError::UnsafeConfinedPath(path.to_owned())),
                }
            }
            Err(colossus_windows_native::WindowsNativeError::Io { source, .. })
                if source.raw_os_error() == Some(WINDOWS_SHARING_VIOLATION) => {}
            Err(_) => return Err(HomeError::UnsafeConfinedPath(path.to_owned())),
        }
        transient_failures += 1;
        if transient_failures < WINDOWS_RACE_RETRIES {
            std::thread::sleep(WINDOWS_RACE_RETRY_DELAY);
        }
    }
    Err(HomeError::UnsafeConfinedPath(path.to_owned()))
}

#[cfg(windows)]
fn open_existing_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<File, HomeError> {
    let path = windows_directory_path(root, parents, false)?.join(leaf);
    let binding =
        colossus_windows_native::BoundPath::open_file(&path).map_err(|error| match error {
            colossus_windows_native::WindowsNativeError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                HomeError::io(&path, source)
            }
            _ => HomeError::UnsafeConfinedPath(path.clone()),
        })?;
    binding
        .validate_private_owner_dacl()
        .and_then(|()| binding.revalidate())
        .map_err(|_| HomeError::UnsafeConfinedPath(path.clone()))?;
    if binding.link_count().ok() != Some(1) {
        return Err(HomeError::UnsafeConfinedPath(path));
    }
    binding
        .try_clone_file()
        .map_err(|_| HomeError::UnsafeConfinedPath(path))
}

#[cfg(windows)]
fn prepare_directory_platform(
    root: &ConfinedRoot,
    components: &[OsString],
) -> Result<(), HomeError> {
    windows_directory_path(root, components, true).map(|_| ())
}

#[cfg(windows)]
fn validate_directory_platform(
    root: &ConfinedRoot,
    components: &[OsString],
) -> Result<(), HomeError> {
    windows_directory_path(root, components, false).map(|_| ())
}

#[cfg(not(any(unix, windows)))]
fn bind_platform_root(path: PathBuf) -> Result<ConfinedRoot, HomeError> {
    Ok(ConfinedRoot { path })
}

#[cfg(not(any(unix, windows)))]
fn revalidate_platform_root(root: &ConfinedRoot) -> Result<(), HomeError> {
    let canonical =
        fs::canonicalize(&root.path).map_err(|error| HomeError::io(&root.path, error))?;
    if canonical != root.path || !canonical.is_dir() {
        return Err(HomeError::UnsafeConfinedPath(root.path.clone()));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn fallback_directory_path(
    root: &ConfinedRoot,
    components: &[OsString],
    create: bool,
) -> Result<PathBuf, HomeError> {
    let path = root.path.join(PathBuf::from_iter(components));
    if create {
        fs::create_dir_all(&path).map_err(|error| HomeError::io(&path, error))?;
    }
    let canonical = fs::canonicalize(&path).map_err(|error| HomeError::io(&path, error))?;
    if canonical != path || !canonical.is_dir() {
        return Err(HomeError::UnsafeConfinedPath(path));
    }
    Ok(path)
}

#[cfg(not(any(unix, windows)))]
fn prepare_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<(), HomeError> {
    let path = fallback_directory_path(root, parents, true)?.join(leaf);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(HomeError::UnsafeConfinedPath(path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(HomeError::io(&path, error)),
    }
}

#[cfg(not(any(unix, windows)))]
fn open_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<(File, bool), HomeError> {
    let path = fallback_directory_path(root, parents, true)?.join(leaf);
    let created = !path.exists();
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| HomeError::io(&path, error))?;
    if !file
        .metadata()
        .map_err(|error| HomeError::io(&path, error))?
        .is_file()
    {
        return Err(HomeError::UnsafeConfinedPath(path));
    }
    Ok((file, created))
}

#[cfg(not(any(unix, windows)))]
fn open_existing_file_platform(
    root: &ConfinedRoot,
    parents: &[OsString],
    leaf: &OsStr,
) -> Result<File, HomeError> {
    let path = fallback_directory_path(root, parents, false)?.join(leaf);
    let file = fs::OpenOptions::new()
        .read(true)
        .open(&path)
        .map_err(|error| HomeError::io(&path, error))?;
    if !file
        .metadata()
        .map_err(|error| HomeError::io(&path, error))?
        .is_file()
    {
        return Err(HomeError::UnsafeConfinedPath(path));
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn prepare_directory_platform(
    root: &ConfinedRoot,
    components: &[OsString],
) -> Result<(), HomeError> {
    fallback_directory_path(root, components, true).map(|_| ())
}

#[cfg(not(any(unix, windows)))]
fn validate_directory_platform(
    root: &ConfinedRoot,
    components: &[OsString],
) -> Result<(), HomeError> {
    fallback_directory_path(root, components, false).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::private_tempdir;

    #[cfg(windows)]
    #[test]
    fn concurrent_private_file_creation_reopens_the_winner() {
        use std::sync::{Arc, Barrier};

        const THREADS: usize = 8;
        let temporary = private_tempdir();
        let root = temporary.path().join("private");
        let confined = ConfinedRoot::bind(&root).expect("confined root");
        let barrier = Arc::new(Barrier::new(THREADS));
        let threads = (0..THREADS)
            .map(|_| {
                let confined = confined.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    confined.open_file(Path::new("race/state.redb"))
                })
            })
            .collect::<Vec<_>>();

        let creators = threads
            .into_iter()
            .map(|thread| {
                thread
                    .join()
                    .expect("file thread")
                    .expect("a concurrent creator must reopen and validate the winning file")
                    .was_created()
            })
            .filter(|created| *created)
            .count();
        assert_eq!(
            creators, 1,
            "exactly one concurrent creator may report creating the file"
        );
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_private_directory_creation_reopens_the_winner() {
        use std::sync::{Arc, Barrier};

        const THREADS: usize = 8;
        let temporary = private_tempdir();
        let root = temporary.path().join("private");
        let confined = ConfinedRoot::bind(&root).expect("confined root");
        let barrier = Arc::new(Barrier::new(THREADS));
        let threads = (0..THREADS)
            .map(|_| {
                let confined = confined.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    confined.prepare_directory(Path::new("race/nested"))
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            assert!(
                thread.join().expect("directory thread").is_ok(),
                "a concurrent creator must reopen and validate the winning directory"
            );
        }
    }

    #[test]
    fn nested_private_paths_are_created_and_reopened_by_descriptor() {
        let temporary = private_tempdir();
        let root = temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root")
            .join("private");
        fs::create_dir(&root).expect("root");
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("permissions");
            let confined = ConfinedRoot::bind(&root).expect("confined root");
            let opened = confined
                .open_file(Path::new("nested/state.redb"))
                .expect("confined file");
            assert!(opened.was_created());
            assert_eq!(
                fs::metadata(root.join("nested"))
                    .expect("nested metadata")
                    .mode()
                    & 0o777,
                0o700
            );
            drop(opened);
            assert!(
                !confined
                    .open_file(Path::new("nested/state.redb"))
                    .expect("reopen")
                    .was_created()
            );
        }
        #[cfg(not(unix))]
        {
            let confined = ConfinedRoot::bind(&root).expect("confined root");
            assert!(
                confined
                    .open_file(Path::new("nested/state.redb"))
                    .expect("confined file")
                    .was_created()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_parent_escape_and_desktop_leaf_alias_are_rejected() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let temporary = private_tempdir();
        let canonical_temporary = temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root");
        let root = canonical_temporary.join("cli");
        let desktop = canonical_temporary.join("desktop");
        fs::create_dir(&root).expect("CLI root");
        fs::create_dir(&desktop).expect("Desktop root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("CLI permissions");
        fs::set_permissions(&desktop, fs::Permissions::from_mode(0o700))
            .expect("Desktop permissions");
        let confined = ConfinedRoot::bind(&root).expect("confined root");

        symlink(&desktop, root.join("escape")).expect("parent symlink");
        assert!(
            confined
                .prepare_file(Path::new("escape/state.redb"))
                .is_err()
        );

        let desktop_state = desktop.join("state.redb");
        fs::write(&desktop_state, []).expect("Desktop state");
        fs::set_permissions(&desktop_state, fs::Permissions::from_mode(0o600))
            .expect("state permissions");
        symlink(&desktop_state, root.join("state.redb")).expect("state symlink");
        assert!(confined.prepare_file(Path::new("state.redb")).is_err());
        assert!(confined.open_file(Path::new("state.redb")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_alias_is_rejected() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = private_tempdir();
        let canonical_temporary = temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root");
        let root = canonical_temporary.join("cli");
        let outside = canonical_temporary.join("desktop.redb");
        fs::create_dir(&root).expect("CLI root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("CLI permissions");
        fs::write(&outside, []).expect("outside state");
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600))
            .expect("outside permissions");
        fs::hard_link(&outside, root.join("state.redb")).expect("hard link");

        let confined = ConfinedRoot::bind(&root).expect("confined root");
        assert!(confined.prepare_file(Path::new("state.redb")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn existing_fifo_fails_without_blocking() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = private_tempdir();
        let canonical_temporary = temporary
            .path()
            .canonicalize()
            .expect("canonical temporary root");
        let root = canonical_temporary.join("home");
        fs::create_dir(&root).expect("home");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("permissions");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join("config.yaml"))
                .status()
                .expect("run mkfifo")
                .success()
        );

        let confined = ConfinedRoot::bind(&root).expect("confined root");
        assert!(
            confined
                .open_existing_file(Path::new("config.yaml"))
                .is_err()
        );
    }
}
