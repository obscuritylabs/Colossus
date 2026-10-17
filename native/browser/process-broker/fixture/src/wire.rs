use crate::inventory::invalid;
use colossus_mac_browser_process_broker::ProcessReceipt;
use std::{
    fs::File,
    io::{self, Read as _, Write as _},
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};
pub const READY: &[u8; 8] = b"CSRD0001";
pub fn read_exact<const N: usize>(stream: &mut UnixStream) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}
pub fn frame(stream: &mut UnixStream, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(invalid("fixture frame bound"));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(bytes)
}
pub fn receive(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let length = u32::from_be_bytes(read_exact(stream)?) as usize;
    if !(1..=4096).contains(&length) {
        return Err(invalid("fixture frame bound"));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}
pub fn receipt(mut file: File) -> io::Result<ProcessReceipt> {
    let flags = rustix::fs::fcntl_getfl(&file)?;
    rustix::fs::fcntl_setfl(&file, flags | rustix::fs::OFlags::NONBLOCK)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 2061];
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&buffer[..n]);
                if bytes.len() > 2060 {
                    return Err(invalid("oversized creator receipt"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(invalid("creator output EOF unknown"));
                }
                let mut wait = [rustix::event::PollFd::new(
                    &file,
                    rustix::event::PollFlags::IN,
                )];
                let timeout =
                    rustix::event::Timespec::try_from(remaining).map_err(io::Error::other)?;
                rustix::event::poll(&mut wait, Some(&timeout))?;
            }
            Err(e) => return Err(e),
        }
    }
    ProcessReceipt::read(&mut bytes.as_slice())
}
