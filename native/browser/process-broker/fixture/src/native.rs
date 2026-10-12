use std::{
    fs::File,
    io,
    os::{
        fd::{AsRawFd as _, FromRawFd as _},
        unix::net::UnixStream,
    },
};
unsafe extern "C" {
    fn colossus_fixture_parent_matches(child: u32, parent: u32) -> i32;
    fn colossus_fixture_send_receipt(channel: i32, receipt: i32) -> i32;
    fn colossus_fixture_receive_receipt(channel: i32) -> i32;
}
pub fn parent_matches(child: u32, parent: u32) -> bool {
    // SAFETY: the native shim reads only one fixed libproc BSD identity record.
    unsafe { colossus_fixture_parent_matches(child, parent) == 1 }
}
pub fn send_receipt(channel: &UnixStream, receipt: &File) -> io::Result<()> {
    // SAFETY: both borrowed live FDs remain valid through the synchronous shim.
    // The shim validates a read-only FIFO and sends exactly that one descriptor.
    if unsafe { colossus_fixture_send_receipt(channel.as_raw_fd(), receipt.as_raw_fd()) } != 0 {
        return Err(io::Error::other("plain receipt transfer rejected"));
    }
    Ok(())
}
pub fn receive_receipt(channel: &UnixStream) -> io::Result<File> {
    // SAFETY: the borrowed live connected fixture channel is valid for this call.
    // The shim rejects truncation/extra capabilities and returns one owned FIFO.
    let raw = unsafe { colossus_fixture_receive_receipt(channel.as_raw_fd()) };
    if raw < 0 {
        return Err(io::Error::other("plain receipt transfer rejected"));
    }
    // SAFETY: successful shim return transfers exclusive ownership, with CLOEXEC.
    Ok(unsafe { File::from_raw_fd(raw) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    #[test]
    fn fixture_transfers_only_one_read_only_plain_pipe_without_stealing_bytes() {
        let mut handles = [-1; 2];
        // SAFETY: pipe writes two descriptor numbers into this fixed writable array.
        assert_eq!(unsafe { libc::pipe(handles.as_mut_ptr()) }, 0);
        // SAFETY: each successful descriptor is uniquely owned by this test.
        let reader = unsafe { File::from_raw_fd(handles[0]) };
        // SAFETY: the distinct successful write descriptor is uniquely owned.
        let mut writer = unsafe { File::from_raw_fd(handles[1]) };
        let (left, right) = UnixStream::pair().unwrap();
        send_receipt(&left, &reader).unwrap();
        let mut admitted = receive_receipt(&right).unwrap();
        writer.write_all(b"finite-receipt").unwrap();
        drop(writer);
        let mut observed = Vec::new();
        admitted.read_to_end(&mut observed).unwrap();
        assert_eq!(observed, b"finite-receipt");
        assert_ne!(
            rustix::io::fcntl_getfd(&admitted).unwrap() & rustix::io::FdFlags::CLOEXEC,
            rustix::io::FdFlags::empty()
        );
    }
    #[test]
    fn write_endpoint_or_control_socket_cannot_be_a_receipt_capability() {
        let (left, right) = UnixStream::pair().unwrap();
        let control = File::from(std::os::fd::OwnedFd::from(right));
        assert!(send_receipt(&left, &control).is_err());
        let mut handles = [-1; 2];
        // SAFETY: the fixed array receives the two owned plain pipe descriptors.
        assert_eq!(unsafe { libc::pipe(handles.as_mut_ptr()) }, 0);
        // SAFETY: pipe succeeded and each distinct descriptor is uniquely owned.
        let _reader = unsafe { File::from_raw_fd(handles[0]) };
        // SAFETY: the successful distinct write descriptor is uniquely owned.
        let writer = unsafe { File::from_raw_fd(handles[1]) };
        assert!(send_receipt(&left, &writer).is_err());
    }
}
