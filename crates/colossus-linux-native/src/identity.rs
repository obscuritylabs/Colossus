use std::fmt;

/// Scoped NFS identity evidence for one already-open filesystem object.
///
/// The server address is returned in network byte order: four bytes for IPv4
/// and sixteen bytes for IPv6. The opaque file handle is returned exactly as
/// supplied by the kernel. It is deliberately omitted from [`Debug`].
#[derive(Clone, Eq, PartialEq)]
pub struct NfsFileIdentity {
    nfs_version: u8,
    server_address: ServerAddress,
    server_port: u16,
    fsid_major: u64,
    fsid_minor: u64,
    handle_type: i32,
    handle: Vec<u8>,
}

impl NfsFileIdentity {
    pub(crate) fn from_consistent_capture(
        before: CaptureSnapshot,
        volume_between_handles: NfsVolumeIdentity,
        after: CaptureSnapshot,
    ) -> std::io::Result<Self> {
        if before.directory != after.directory
            || before.volume != volume_between_handles
            || volume_between_handles != after.volume
            || before.file_handle != after.file_handle
        {
            return Err(inconsistent_capture());
        }

        Ok(Self {
            nfs_version: before.volume.nfs_version,
            server_address: before.volume.server_address,
            server_port: before.volume.server_port,
            fsid_major: before.volume.fsid_major,
            fsid_minor: before.volume.fsid_minor,
            handle_type: before.file_handle.handle_type,
            handle: before.file_handle.handle,
        })
    }

    /// NFS protocol major version reported by the kernel.
    pub const fn nfs_version(&self) -> u8 {
        self.nfs_version
    }

    /// Binary server address in network byte order.
    ///
    /// Its length is exactly four for IPv4 or sixteen for IPv6.
    pub fn server_address(&self) -> &[u8] {
        self.server_address.as_bytes()
    }

    /// Server port reported by the kernel.
    pub const fn server_port(&self) -> u16 {
        self.server_port
    }

    /// Major component of the NFS server filesystem identifier.
    pub const fn fsid_major(&self) -> u64 {
        self.fsid_major
    }

    /// Minor component of the NFS server filesystem identifier.
    pub const fn fsid_minor(&self) -> u64 {
        self.fsid_minor
    }

    /// Kernel-defined type tag for the opaque file handle.
    pub const fn handle_type(&self) -> i32 {
        self.handle_type
    }

    /// Exact opaque file-handle bytes supplied by the kernel.
    pub fn handle(&self) -> &[u8] {
        &self.handle
    }
}

impl fmt::Debug for NfsFileIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NfsFileIdentity")
            .field("nfs_version", &self.nfs_version)
            .field("server_address", &self.server_address)
            .field("server_port", &self.server_port)
            .field("fsid_major", &self.fsid_major)
            .field("fsid_minor", &self.fsid_minor)
            .field("handle_type", &self.handle_type)
            .field("handle_len", &self.handle.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerAddress {
    Ipv4([u8; 4]),
    Ipv6([u8; 16]),
}

impl ServerAddress {
    pub(crate) const fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Ipv4(address) => address,
            Self::Ipv6(address) => address,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NfsVolumeIdentity {
    pub(crate) nfs_version: u8,
    pub(crate) server_address: ServerAddress,
    pub(crate) server_port: u16,
    pub(crate) fsid_major: u64,
    pub(crate) fsid_minor: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct DirectoryIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct CapturedFileHandle {
    pub(crate) mount_id: i32,
    pub(crate) handle_type: i32,
    pub(crate) handle: Vec<u8>,
}

pub(crate) struct CaptureSnapshot {
    pub(crate) directory: DirectoryIdentity,
    pub(crate) volume: NfsVolumeIdentity,
    pub(crate) file_handle: CapturedFileHandle,
}

fn inconsistent_capture() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "Linux NFS identity evidence changed during capture",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureSnapshot, CapturedFileHandle, DirectoryIdentity, NfsFileIdentity, NfsVolumeIdentity,
        ServerAddress,
    };

    fn volume() -> NfsVolumeIdentity {
        NfsVolumeIdentity {
            nfs_version: 4,
            server_address: ServerAddress::Ipv4([192, 0, 2, 1]),
            server_port: 2049,
            fsid_major: 17,
            fsid_minor: 29,
        }
    }

    fn file_handle() -> CapturedFileHandle {
        CapturedFileHandle {
            mount_id: 41,
            handle_type: 7,
            handle: vec![222, 173, 190, 239],
        }
    }

    fn snapshot() -> CaptureSnapshot {
        CaptureSnapshot {
            directory: DirectoryIdentity {
                device: 23,
                inode: 101,
            },
            volume: volume(),
            file_handle: file_handle(),
        }
    }

    fn consistent_identity() -> NfsFileIdentity {
        NfsFileIdentity::from_consistent_capture(snapshot(), volume(), snapshot()).unwrap()
    }

    #[test]
    fn debug_redacts_the_opaque_handle() {
        let identity = consistent_identity();

        let rendered = format!("{identity:?}");
        assert!(rendered.contains("handle_len: 4"));
        assert!(!rendered.contains("222"));
        assert!(!rendered.contains("173"));
        assert!(!rendered.contains("190"));
        assert!(!rendered.contains("239"));
    }

    #[test]
    fn consistent_capture_preserves_exact_stable_evidence() {
        let identity = consistent_identity();

        assert_eq!(identity.nfs_version(), 4);
        assert_eq!(identity.server_address(), [192, 0, 2, 1]);
        assert_eq!(identity.server_port(), 2049);
        assert_eq!(identity.fsid_major(), 17);
        assert_eq!(identity.fsid_minor(), 29);
        assert_eq!(identity.handle_type(), 7);
        assert_eq!(identity.handle(), [222, 173, 190, 239]);
    }

    #[test]
    fn association_rejects_device_or_inode_changes() {
        let mut changed_device = snapshot();
        changed_device.directory.device += 1;
        assert!(
            NfsFileIdentity::from_consistent_capture(snapshot(), volume(), changed_device).is_err()
        );

        let mut changed_inode = snapshot();
        changed_inode.directory.inode += 1;
        assert!(
            NfsFileIdentity::from_consistent_capture(snapshot(), volume(), changed_inode).is_err()
        );
    }

    #[test]
    fn association_rejects_any_volume_change() {
        let mut changed_volume = volume();
        changed_volume.fsid_minor += 1;
        assert!(
            NfsFileIdentity::from_consistent_capture(snapshot(), changed_volume, snapshot(),)
                .is_err()
        );

        let mut changed_after = snapshot();
        changed_after.volume.server_port += 1;
        assert!(
            NfsFileIdentity::from_consistent_capture(snapshot(), volume(), changed_after).is_err()
        );
    }

    #[test]
    fn association_rejects_mount_type_or_handle_changes() {
        let mut changed_mount = snapshot();
        changed_mount.file_handle.mount_id += 1;
        assert!(
            NfsFileIdentity::from_consistent_capture(snapshot(), volume(), changed_mount).is_err()
        );

        let mut changed_type = snapshot();
        changed_type.file_handle.handle_type += 1;
        assert!(
            NfsFileIdentity::from_consistent_capture(snapshot(), volume(), changed_type).is_err()
        );

        let mut changed_handle = snapshot();
        changed_handle.file_handle.handle[0] ^= 0xff;
        assert!(
            NfsFileIdentity::from_consistent_capture(snapshot(), volume(), changed_handle).is_err()
        );
    }
}
