//! Descriptor-relative metadata census for an opaque macOS Chromium allocation.
//!
//! Chromium creates sockets and symbolic locks in its temporary profile. Neither
//! those entries nor signed framework links are followed or read. Every model
//! filesystem/process effect gets a fresh bounded census and revalidates it.
use super::*;
use std::{
    ffi::OsString,
    fs::File,
    os::unix::{ffi::OsStringExt as _, fs::MetadataExt as _},
};

const MAX_ENTRIES: usize = 8192;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Eq, PartialEq)]
struct Identity {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
    links: u64,
}

impl Identity {
    fn metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
            mode: metadata.mode(),
            links: metadata.nlink(),
        }
    }

    fn stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: stat.st_dev as u64,
            inode: stat.st_ino,
            owner: stat.st_uid,
            mode: stat.st_mode as u32,
            links: stat.st_nlink as u64,
        }
    }
}

struct Node {
    parent: Arc<File>,
    name: OsString,
    // Only ordinary files and directories are opened. Socket and symlink
    // identities are retained through their parent descriptor plus fstatat.
    handle: Option<Arc<File>>,
    identity: Identity,
    regular: bool,
}

pub(super) struct Snapshot {
    roots: Vec<ConfinedRoot>,
    nodes: Vec<Node>,
    listings: Vec<(Arc<File>, Vec<OsString>)>,
}

impl Snapshot {
    pub(super) fn capture(roots: &[ConfinedRoot]) -> Result<Self, ExecutionError> {
        let mut result = Self {
            roots: roots.to_vec(),
            nodes: Vec::new(),
            listings: Vec::new(),
        };
        for root in roots {
            root.revalidate().map_err(protected_failure)?;
            let directory = Arc::new(root.directory_handle().map_err(protected_failure)?);
            let mut directories = vec![(directory, 0_usize)];
            while let Some((directory, depth)) = directories.pop() {
                if depth > MAX_DEPTH {
                    return Err(protected_failure("native cache depth bound"));
                }
                let mut names = Vec::new();
                for entry in rustix::fs::Dir::read_from(&directory).map_err(protected_failure)? {
                    let entry = entry.map_err(protected_failure)?;
                    let name = entry.file_name().to_bytes();
                    if matches!(name, b"." | b"..") {
                        continue;
                    }
                    if result.nodes.len() >= MAX_ENTRIES {
                        return Err(protected_failure("native cache metadata bound"));
                    }
                    let name = OsString::from_vec(name.to_vec());
                    names.push(name.clone());
                    let before = rustix::fs::statat(
                        &directory,
                        &name,
                        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
                    )
                    .map_err(protected_failure)?;
                    let identity = Identity::stat(&before);
                    let kind = identity.mode & libc::S_IFMT as u32;
                    let directory_entry = kind == libc::S_IFDIR as u32;
                    let regular = kind == libc::S_IFREG as u32;
                    if identity.owner != rustix::process::geteuid().as_raw()
                        || regular && identity.links != 1
                        || !(directory_entry
                            || regular
                            || kind == libc::S_IFLNK as u32
                            || kind == libc::S_IFSOCK as u32)
                    {
                        return Err(protected_failure("unknown native cache object"));
                    }
                    let handle = if directory_entry || regular {
                        let mut flags = rustix::fs::OFlags::RDONLY
                            | rustix::fs::OFlags::NOFOLLOW
                            | rustix::fs::OFlags::CLOEXEC;
                        if directory_entry {
                            flags |= rustix::fs::OFlags::DIRECTORY;
                        }
                        let opened =
                            rustix::fs::openat(&directory, &name, flags, rustix::fs::Mode::empty())
                                .map_err(protected_failure)?;
                        let opened = Arc::new(File::from(opened));
                        if Identity::metadata(&opened.metadata().map_err(protected_failure)?)
                            != identity
                        {
                            return Err(protected_failure("native cache object changed"));
                        }
                        if directory_entry {
                            directories.push((Arc::clone(&opened), depth + 1));
                        }
                        Some(opened)
                    } else {
                        None
                    };
                    let after = rustix::fs::statat(
                        &directory,
                        &name,
                        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
                    )
                    .map_err(protected_failure)?;
                    if Identity::stat(&after) != identity {
                        return Err(protected_failure("native cache object changed"));
                    }
                    result.nodes.push(Node {
                        parent: Arc::clone(&directory),
                        name,
                        handle,
                        identity,
                        regular,
                    });
                }
                names.sort();
                result.listings.push((directory, names));
            }
            root.revalidate().map_err(protected_failure)?;
        }
        result.revalidate()?;
        Ok(result)
    }

    pub(super) fn check_file(&self, file: &File) -> Result<(), ExecutionError> {
        let identity = Identity::metadata(&file.metadata().map_err(protected_failure)?);
        if self.nodes.iter().any(|node| {
            node.regular
                && node.identity.device == identity.device
                && node.identity.inode == identity.inode
        }) {
            return Err(adapter_failure(
                "native browser profile is unavailable to filesystem effects",
            ));
        }
        Ok(())
    }

    pub(super) fn revalidate(&self) -> Result<(), ExecutionError> {
        for root in &self.roots {
            root.revalidate().map_err(protected_failure)?;
        }
        let mut count = 0_usize;
        for (directory, expected) in &self.listings {
            let mut actual = Vec::new();
            for entry in rustix::fs::Dir::read_from(directory).map_err(protected_failure)? {
                let entry = entry.map_err(protected_failure)?;
                let name = entry.file_name().to_bytes();
                if matches!(name, b"." | b"..") {
                    continue;
                }
                count += 1;
                if count > MAX_ENTRIES {
                    return Err(protected_failure("native cache metadata bound"));
                }
                actual.push(OsString::from_vec(name.to_vec()));
            }
            actual.sort();
            if &actual != expected {
                return Err(protected_failure("native cache directory entries changed"));
            }
        }
        for node in &self.nodes {
            let current = rustix::fs::statat(
                &node.parent,
                &node.name,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(protected_failure)?;
            if Identity::stat(&current) != node.identity
                || node.handle.as_ref().is_some_and(|handle| {
                    handle.metadata().map_or(true, |metadata| {
                        Identity::metadata(&metadata) != node.identity
                    })
                })
            {
                return Err(protected_failure("native cache identity changed"));
            }
        }
        for root in &self.roots {
            root.revalidate().map_err(protected_failure)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "native_profiles/tests.rs"]
mod tests;
