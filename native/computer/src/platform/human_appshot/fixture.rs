//! Metadata-only admission for explicitly invoked synthetic App Shot acceptance.
use super::{Error, identity, process_image};
use crate::{FixtureAppshotIdentity, ForegroundAppshot, HumanAppshot, NativeProcessIdentity};
use colossus_windows_native::{BoundPath, FileIdentity};
use sha2::{Digest as _, Sha256};
use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
};
use windows_sys::Win32::System::Threading::{
    GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

const MAX_FIXTURE_IMAGE: u64 = 16 * 1024 * 1024;

fn reviewed_image(image: &BoundPath, expected_sha256: &str) -> Result<File, Error> {
    if expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::InvalidInput);
    }
    image.revalidate().map_err(|_| Error::Denied)?;
    let mut file = image.try_clone_file().map_err(|_| Error::Denied)?;
    if file.metadata().map_err(|_| Error::Denied)?.len() > MAX_FIXTURE_IMAGE {
        return Err(Error::Denied);
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| Error::Denied)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| Error::Denied)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_FIXTURE_IMAGE {
            return Err(Error::Denied);
        }
        hash.update(&buffer[..count]);
    }
    if format!("{:x}", hash.finalize()) != expected_sha256 {
        return Err(Error::Denied);
    }
    image.revalidate().map_err(|_| Error::Denied)?;
    Ok(file)
}

fn query_process(pid: u32) -> Result<OwnedHandle, Error> {
    // SAFETY: query-only handle for the already frozen native process identity.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(Error::Denied);
    }
    // SAFETY: successful OpenProcess transfers the exact retained query handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
}

fn require_process_image(
    process: &OwnedHandle,
    expected_process: &NativeProcessIdentity,
    expected_image: FileIdentity,
) -> Result<(), Error> {
    // SAFETY: retained query-only process handle, never a renderer-supplied PID.
    let pid = unsafe { GetProcessId(process.as_raw_handle().cast()) };
    if pid != expected_process.pid || identity::process(pid)? != *expected_process {
        return Err(Error::Stale);
    }
    let actual_image = process_image(process)?;
    if actual_image.identity() != expected_image {
        return Err(Error::Denied);
    }
    actual_image.revalidate().map_err(|_| Error::Denied)?;
    if identity::process(pid)? != *expected_process {
        return Err(Error::Stale);
    }
    Ok(())
}

pub(crate) fn admit(
    ticket: &ForegroundAppshot,
    expected_image: &BoundPath,
    expected_sha256: &str,
) -> Result<FixtureAppshotIdentity, Error> {
    let image = reviewed_image(expected_image, expected_sha256)?;
    ticket.window.revalidate(&ticket.context)?;
    let process = query_process(ticket.window.process.pid)?;
    require_process_image(&process, &ticket.window.process, expected_image.identity())?;
    ticket.window.revalidate(&ticket.context)?;
    Ok(FixtureAppshotIdentity {
        window: ticket.window.clone(),
        process,
        image,
        image_identity: expected_image.identity(),
    })
}

pub(crate) fn capture(
    ticket: ForegroundAppshot,
    include_text: bool,
    admitted: &FixtureAppshotIdentity,
) -> Result<HumanAppshot, Error> {
    if ticket.window != admitted.window
        || FileIdentity::of(&admitted.image).map_err(|_| Error::Denied)? != admitted.image_identity
    {
        return Err(Error::Stale);
    }
    ticket.window.revalidate(&ticket.context)?;
    require_process_image(
        &admitted.process,
        &ticket.window.process,
        admitted.image_identity,
    )?;
    ticket.window.revalidate(&ticket.context)?;
    super::capture(ticket, include_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_image_admission_rejects_other_images_and_stale_birth_without_capture() {
        let pid = std::process::id();
        let process = query_process(pid).unwrap();
        let actual = identity::process(pid).unwrap();
        let image = BoundPath::open_immutable_file(&std::env::current_exe().unwrap()).unwrap();
        assert!(require_process_image(&process, &actual, image.identity()).is_ok());
        let other = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("kernel32.dll");
        let other = BoundPath::open_immutable_file(&other).unwrap();
        assert_ne!(image.identity(), other.identity());
        assert!(require_process_image(&process, &actual, other.identity()).is_err());
        let mut stale = actual;
        stale.birth ^= 1;
        assert!(require_process_image(&process, &stale, image.identity()).is_err());
    }

    #[test]
    fn reviewed_image_rejects_wrong_or_malformed_digest_without_capture() {
        let path = std::env::current_exe().unwrap();
        let image = BoundPath::open_immutable_file(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let expected = format!("{:x}", Sha256::digest(&bytes));
        assert!(reviewed_image(&image, &expected).is_ok());
        assert!(reviewed_image(&image, &"0".repeat(64)).is_err());
        assert!(reviewed_image(&image, "title-only").is_err());
    }
}
