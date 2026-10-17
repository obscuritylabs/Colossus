//! Opaque Chromium cache metadata; no child link is followed and no contents read.
use super::*;
use std::{
    ffi::OsString,
    fs::File,
    os::unix::{
        ffi::OsStringExt as _,
        fs::{FileTypeExt as _, MetadataExt as _},
    },
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
    // Native Linux stat uses u64 links on x86_64 and u32 on aarch64/riscv64.
    #[allow(clippy::useless_conversion)]
    fn stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
            owner: stat.st_uid,
            mode: stat.st_mode,
            links: u64::from(stat.st_nlink),
        }
    }
}
struct Node {
    parent: Arc<File>,
    name: OsString,
    handle: File,
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
            let file = root.directory_handle().map_err(protected_failure)?;
            let mut directories = vec![(Arc::new(file), 0)];
            while let Some((directory, depth)) = directories.pop() {
                if depth > MAX_DEPTH {
                    return Err(protected_failure("native cache depth bound"));
                }
                let listing = rustix::fs::Dir::read_from(&directory).map_err(protected_failure)?;
                let mut names = Vec::new();
                for entry in listing {
                    let entry = entry.map_err(protected_failure)?;
                    let bytes = entry.file_name().to_bytes();
                    if matches!(bytes, b"." | b"..") {
                        continue;
                    }
                    if result.nodes.len() >= MAX_ENTRIES {
                        return Err(protected_failure("native cache metadata bound"));
                    }
                    let name = OsString::from_vec(bytes.to_vec());
                    names.push(name.clone());
                    let descriptor = rustix::fs::openat(
                        &directory,
                        &name,
                        rustix::fs::OFlags::PATH
                            | rustix::fs::OFlags::NOFOLLOW
                            | rustix::fs::OFlags::CLOEXEC,
                        rustix::fs::Mode::empty(),
                    )
                    .map_err(protected_failure)?;
                    let handle = File::from(descriptor);
                    let metadata = handle.metadata().map_err(protected_failure)?;
                    let kind = metadata.file_type();
                    if metadata.uid() != rustix::process::geteuid().as_raw()
                        || (kind.is_file() && metadata.nlink() != 1)
                        || !(kind.is_dir()
                            || kind.is_file()
                            || kind.is_symlink()
                            || kind.is_socket())
                    {
                        return Err(protected_failure("unknown native cache object"));
                    }
                    let identity = Identity::metadata(&metadata);
                    let current = rustix::fs::statat(
                        &directory,
                        &name,
                        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
                    )
                    .map_err(protected_failure)?;
                    if Identity::stat(&current) != identity {
                        return Err(protected_failure("native cache object changed"));
                    }
                    if kind.is_dir() {
                        let child = rustix::fs::openat(
                            &directory,
                            &name,
                            rustix::fs::OFlags::RDONLY
                                | rustix::fs::OFlags::DIRECTORY
                                | rustix::fs::OFlags::NOFOLLOW
                                | rustix::fs::OFlags::CLOEXEC,
                            rustix::fs::Mode::empty(),
                        )
                        .map_err(protected_failure)?;
                        let child = File::from(child);
                        if Identity::metadata(&child.metadata().map_err(protected_failure)?)
                            != identity
                        {
                            return Err(protected_failure("native cache directory changed"));
                        }
                        directories.push((Arc::new(child), depth + 1));
                    }
                    result.nodes.push(Node {
                        parent: Arc::clone(&directory),
                        name,
                        handle,
                        identity,
                        regular: kind.is_file(),
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
                let bytes = entry.file_name().to_bytes();
                if matches!(bytes, b"." | b"..") {
                    continue;
                }
                count += 1;
                if count > MAX_ENTRIES {
                    return Err(protected_failure("native cache metadata bound"));
                }
                actual.push(OsString::from_vec(bytes.to_vec()));
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
                || Identity::metadata(&node.handle.metadata().map_err(protected_failure)?)
                    != node.identity
            {
                return Err(protected_failure("native cache identity changed"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
