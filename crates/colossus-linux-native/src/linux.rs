use crate::{
    identity::{
        CaptureSnapshot, CapturedFileHandle, DirectoryIdentity, NfsFileIdentity, NfsVolumeIdentity,
    },
    volume::{MAX_NFS_VOLUMES_BYTES, parse_matching_volume},
};
use std::{
    fs::File,
    io::{self, Read as _},
    mem::{align_of, offset_of, size_of},
    os::fd::{AsFd, AsRawFd, BorrowedFd},
};

const NFS_VOLUMES_PATH: &str = "/proc/fs/nfsfs/volumes";
const MAX_HANDLE_BYTES: usize = 128;

#[repr(C)]
struct FileHandleBuffer {
    handle_bytes: libc::c_uint,
    handle_type: libc::c_int,
    bytes: [libc::c_uchar; MAX_HANDLE_BYTES],
}

const _: () = {
    assert!(offset_of!(FileHandleBuffer, bytes) == size_of::<libc::file_handle>());
    assert!(align_of::<FileHandleBuffer>() >= align_of::<libc::file_handle>());
};

/// Capture scoped NFS identity evidence from an already-open directory.
///
/// The descriptor is never closed or reopened. The operation rejects non-directory
/// descriptors, unsupported file handles, malformed or oversized kernel volume
/// metadata, and device identifiers that do not match exactly one live NFS volume.
pub fn capture_nfs_file_identity(directory: impl AsFd) -> io::Result<NfsFileIdentity> {
    let directory = directory.as_fd();
    let metadata_before = directory_metadata(directory)?;
    let device_major = rustix::fs::major(metadata_before.st_dev);
    let device_minor = rustix::fs::minor(metadata_before.st_dev);
    let volume_before = read_matching_volume(device_major, device_minor)?;
    let file_handle_before = capture_file_handle(directory)?;
    let volume_between_handles = read_matching_volume(device_major, device_minor)?;
    let file_handle_after = capture_file_handle(directory)?;
    let metadata_after = directory_metadata(directory)?;
    let volume_after = read_matching_volume(
        rustix::fs::major(metadata_after.st_dev),
        rustix::fs::minor(metadata_after.st_dev),
    )?;

    NfsFileIdentity::from_consistent_capture(
        CaptureSnapshot {
            directory: metadata_identity(&metadata_before),
            volume: volume_before,
            file_handle: file_handle_before,
        },
        volume_between_handles,
        CaptureSnapshot {
            directory: metadata_identity(&metadata_after),
            volume: volume_after,
            file_handle: file_handle_after,
        },
    )
}

const fn metadata_identity(metadata: &rustix::fs::Stat) -> DirectoryIdentity {
    DirectoryIdentity {
        device: metadata.st_dev,
        inode: metadata.st_ino,
    }
}

fn directory_metadata(directory: BorrowedFd<'_>) -> io::Result<rustix::fs::Stat> {
    let metadata = rustix::fs::fstat(directory).map_err(io::Error::from)?;
    if rustix::fs::FileType::from_raw_mode(metadata.st_mode) != rustix::fs::FileType::Directory {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NFS identity requires an open directory",
        ));
    }
    Ok(metadata)
}

fn read_matching_volume(device_major: u32, device_minor: u32) -> io::Result<NfsVolumeIdentity> {
    let volumes = read_nfs_volumes()?;
    parse_matching_volume(&volumes, device_major, device_minor)
}

fn capture_file_handle(directory: BorrowedFd<'_>) -> io::Result<CapturedFileHandle> {
    let mut buffer = FileHandleBuffer {
        handle_bytes: MAX_HANDLE_BYTES as libc::c_uint,
        handle_type: 0,
        bytes: [0; MAX_HANDLE_BYTES],
    };
    let mut mount_id = 0;

    // SAFETY: `directory` remains borrowed for the call; the pathname is a static,
    // NUL-terminated empty C string; `buffer` has the exact C header followed by
    // `MAX_HANDLE_BYTES` writable bytes; and `mount_id` is a valid output pointer.
    let result = unsafe {
        libc::name_to_handle_at(
            directory.as_raw_fd(),
            c"".as_ptr(),
            std::ptr::from_mut(&mut buffer).cast::<libc::file_handle>(),
            &mut mount_id,
            libc::AT_EMPTY_PATH,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    let handle = validated_handle_bytes(buffer.handle_bytes, &buffer.bytes)?;
    if mount_id <= 0 {
        return Err(invalid_handle());
    }
    Ok(CapturedFileHandle {
        mount_id,
        handle_type: buffer.handle_type,
        handle,
    })
}

fn validated_handle_bytes(handle_bytes: libc::c_uint, bytes: &[u8]) -> io::Result<Vec<u8>> {
    let handle_len = usize::try_from(handle_bytes).map_err(|_| invalid_handle())?;
    if handle_len == 0 || handle_len > MAX_HANDLE_BYTES || !handle_len.is_multiple_of(4) {
        return Err(invalid_handle());
    }
    let handle = bytes.get(..handle_len).ok_or_else(invalid_handle)?;
    Ok(handle.to_vec())
}

fn read_nfs_volumes() -> io::Result<Vec<u8>> {
    let file = File::open(NFS_VOLUMES_PATH)?;
    let mut contents = Vec::with_capacity(4096);
    file.take((MAX_NFS_VOLUMES_BYTES + 1) as u64)
        .read_to_end(&mut contents)?;
    if contents.len() > MAX_NFS_VOLUMES_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux NFS volume table exceeds the safety limit",
        ));
    }
    Ok(contents)
}

fn invalid_handle() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Linux returned an invalid opaque file-handle length",
    )
}

#[cfg(test)]
mod tests {
    use super::{MAX_HANDLE_BYTES, validated_handle_bytes};

    #[test]
    fn handle_validator_rejects_zero_length() {
        assert!(validated_handle_bytes(0, &[0; MAX_HANDLE_BYTES]).is_err());
    }

    #[test]
    fn handle_validator_rejects_non_word_length() {
        assert!(validated_handle_bytes(6, &[0; MAX_HANDLE_BYTES]).is_err());
    }

    #[test]
    fn handle_validator_rejects_length_over_the_limit() {
        assert!(validated_handle_bytes(132, &[0; MAX_HANDLE_BYTES]).is_err());
    }

    #[test]
    fn handle_validator_copies_exact_reported_bytes() {
        let mut source = [0_u8; MAX_HANDLE_BYTES];
        source[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        source[8] = 99;

        assert_eq!(
            validated_handle_bytes(8, &source).unwrap(),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
    }
}
