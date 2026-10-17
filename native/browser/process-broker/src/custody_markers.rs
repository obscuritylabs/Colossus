//! Fixed disposable public marker ownership, separate from real PKI state.
use crate::arguments::invalid;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};
const CONTENT: &[u8] = b"colossus public custody probe v1\n";
const NAMES: [&str; 5] = [
    ".colossus-custody-probe-read",
    ".colossus-custody-probe-write",
    ".colossus-custody-probe-rename",
    ".colossus-custody-probe-unlink",
    ".colossus-custody-probe-link",
];
struct Marker {
    path: PathBuf,
    file: File,
    device: u64,
    inode: u64,
}
pub(crate) struct Markers {
    root: PathBuf,
    parent: File,
    device: u64,
    inode: u64,
    markers: Vec<Marker>,
    finished: bool,
}
impl Markers {
    pub fn prepare(main: &Path, policy: [u8; 32]) -> io::Result<Self> {
        let (root, parent) = namespace(main, policy)?;
        let value = parent.metadata()?;
        let mut owner = Self {
            root,
            parent,
            device: value.dev(),
            inode: value.ino(),
            markers: Vec::with_capacity(5),
            finished: false,
        };
        // No destructor prunes a partial or failed preparation. Keep every exact
        // file handle until explicit generation-verified cleanup succeeds.
        for name in NAMES {
            owner.revalidate()?;
            let path = owner.root.join(name);
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(
                    (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC).bits() as i32,
                )
                .open(&path)?;
            let value = file.metadata()?;
            owner.markers.push(Marker {
                path,
                file: file.try_clone()?,
                device: value.dev(),
                inode: value.ino(),
            });
            file.write_all(CONTENT)?;
            file.sync_all()?;
        }
        for name in [".colossus-custody-probe-create"] {
            if owner.root.join(name).symlink_metadata().is_ok() {
                return Err(invalid("custody probe destination already exists"));
            }
        }
        owner.parent.sync_all()?;
        owner.revalidate()?;
        Ok(owner)
    }
    pub fn revalidate(&self) -> io::Result<()> {
        let parent = self.parent.metadata()?;
        let entry = self.root.symlink_metadata()?;
        if entry.file_type().is_symlink()
            || (parent.dev(), parent.ino()) != (self.device, self.inode)
            || (entry.dev(), entry.ino()) != (self.device, self.inode)
        {
            return Err(invalid("custody marker parent changed"));
        }
        match self
            .root
            .join(".colossus-custody-probe-create")
            .symlink_metadata()
        {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return Err(invalid("custody probe created an unexpected file")),
        }
        for marker in &self.markers {
            let held = marker.file.metadata()?;
            let entry = marker.path.symlink_metadata()?;
            if entry.file_type().is_symlink()
                || !held.is_file()
                || held.nlink() != 1
                || held.mode() & 0o7777 != 0o600
                || (held.dev(), held.ino()) != (marker.device, marker.inode)
                || (entry.dev(), entry.ino()) != (marker.device, marker.inode)
                || held.len() != CONTENT.len() as u64
            {
                return Err(invalid("custody marker generation changed"));
            }
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                .open(&marker.path)?;
            let mut bytes = Vec::new();
            file.take(CONTENT.len() as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes != CONTENT {
                return Err(invalid("custody marker bytes changed"));
            }
        }
        Ok(())
    }
    pub fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.revalidate()?;
        for marker in &self.markers {
            std::fs::remove_file(&marker.path)?;
        }
        self.parent.sync_all()?;
        self.finished = true;
        Ok(())
    }
}

/// Resolve only the exact trusted policy namespace; no marker or OS-store mutation.
pub(crate) fn namespace(main: &Path, policy: [u8; 32]) -> io::Result<(PathBuf, File)> {
    let app = main
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| invalid("fixed Main package missing"))?;
    if main != app.join("Contents/MacOS/colossus-native-browser-host") {
        return Err(invalid("nonfixed Main for custody markers"));
    }
    let path = app.join("Contents/Resources/colossus-network-envelope.policy");
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)?;
    let before = file.metadata()?;
    if !before.is_file()
        || before.nlink() != 1
        || before.len() > 32768
        || before.mode() & 0o7777 != 0o400
    {
        return Err(invalid("invalid custody policy resource"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(32769)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != before.len() || Sha256::digest(&bytes).as_slice() != policy {
        return Err(invalid("custody policy resource changed"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid("nonUTF8 custody policy"))?;
    let fields: Vec<_> = text.split_terminator('\n').collect();
    if fields.len() != 9
        || fields[0] != "COLOSSUS_MAC_NETWORK_ENVELOPE_V1"
        || Some(fields[4]) != app.to_str()
    {
        return Err(invalid("custody policy differs from fixed package"));
    }
    let root = PathBuf::from(fields[7]);
    if !root.is_absolute()
        || root.canonicalize()? != root
        || root.starts_with(app)
        || app.starts_with(&root)
    {
        return Err(invalid("custody namespace is not independent"));
    }
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags(
            (rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC)
                .bits() as i32,
        )
        .open(&root)?;
    let value = parent.metadata()?;
    if !value.is_dir() || value.mode() & 0o7777 != 0o700 || value.uid() != before.uid() {
        return Err(invalid("invalid custody marker namespace"));
    }
    Ok((root, parent))
}
