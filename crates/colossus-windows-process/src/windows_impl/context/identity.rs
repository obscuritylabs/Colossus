//! Private kernel identities for the fixed browser-context transaction.
use super::super::{OwnedHandle, WindowsProcessError, last_error};
use colossus_windows_native::{BoundPath, FileIdentity};
use std::{
    ffi::OsString,
    mem::size_of,
    os::windows::ffi::OsStringExt as _,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{
        DuplicateHandle, ERROR_INSUFFICIENT_BUFFER, FILETIME, GetLastError, HANDLE, LocalFree,
    },
    Security::{
        Authorization::ConvertSidToStringSidW, CreateRestrictedToken, DISABLE_MAX_PRIVILEGE,
        EqualSid, GetTokenInformation, IsValidSid, LUA_TOKEN, SID_AND_ATTRIBUTES,
        TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_GROUPS, TOKEN_QUERY,
        TokenAppContainerSid, TokenCapabilities, TokenElevation, TokenIntegrityLevel,
        TokenIsAppContainer, TokenSessionId, TokenStatistics, TokenUser,
    },
    System::Threading::{
        GetCurrentProcess, GetProcessId, GetProcessTimes, OpenProcessToken,
        QueryFullProcessImageNameW, ResumeThread,
    },
};

pub(super) fn require(value: bool, label: &'static str) -> Result<(), WindowsProcessError> {
    if value {
        Ok(())
    } else {
        Err(WindowsProcessError::Invalid(label.into()))
    }
}

pub(super) fn native<T>(
    result: Result<T, colossus_windows_native::WindowsNativeError>,
) -> Result<T, WindowsProcessError> {
    result.map_err(|error| {
        super::diagnostics::record_native(&error);
        WindowsProcessError::Invalid("native browser context identity rejected".into())
    })
}

pub(super) struct Token {
    pub user: Vec<u8>,
    pub session: u32,
    pub authentication: [u32; 2],
    pub elevated: bool,
    pub appcontainer: bool,
    pub integrity: u32,
    pub capabilities: u32,
    pub package: Option<Vec<u8>>,
}

fn token_data(token: HANDLE, kind: i32) -> Result<(Vec<usize>, usize), WindowsProcessError> {
    let mut needed = 0;
    // SAFETY: sizing query uses no buffer and retains the exact token.
    let sized = unsafe { GetTokenInformation(token, kind, null_mut(), 0, &mut needed) };
    // SAFETY: capture the sizing call's thread-local error immediately.
    let error = unsafe { GetLastError() };
    if sized == 0 && error != ERROR_INSUFFICIENT_BUFFER {
        super::diagnostics::record_sizing(kind, error);
    }
    require(
        sized == 0 && error == ERROR_INSUFFICIENT_BUFFER,
        "token sizing query rejected",
    )?;
    require(
        needed > 0 && needed <= 65_536,
        "token information bound rejected",
    )?;
    let mut storage = vec![0_usize; (needed as usize).div_ceil(size_of::<usize>())];
    let mut returned = 0;
    // SAFETY: storage is aligned, initialized and at least the queried byte size.
    if unsafe {
        GetTokenInformation(
            token,
            kind,
            storage.as_mut_ptr().cast(),
            needed,
            &mut returned,
        )
    } == 0
    {
        return Err(last_error("GetTokenInformation(browser context)"));
    }
    require(returned <= needed, "token information overflow")?;
    Ok((storage, returned as usize))
}

fn integer(token: HANDLE, kind: i32) -> Result<u32, WindowsProcessError> {
    let (storage, length) = token_data(token, kind)?;
    require(length >= 4, "truncated token integer")?;
    // SAFETY: storage holds at least four initialized bytes; unaligned read is explicit.
    Ok(unsafe { std::ptr::read_unaligned(storage.as_ptr().cast::<u32>()) })
}

fn sid(token: HANDLE, kind: i32) -> Result<Vec<u8>, WindowsProcessError> {
    let (storage, length) = token_data(token, kind)?;
    require(length >= size_of::<usize>(), "truncated token SID pointer")?;
    // SAFETY: successful token output contains a pointer-sized first field.
    let pointer = unsafe { std::ptr::read_unaligned(storage.as_ptr().cast::<*const u8>()) };
    let start = storage.as_ptr() as usize;
    let at = pointer as usize;
    require(
        at >= start && at.checked_add(8).is_some_and(|end| end <= start + length),
        "token SID outside buffer",
    )?;
    // SAFETY: the eight-byte SID header lies in the returned token buffer.
    let count = unsafe { *pointer.add(1) } as usize;
    let bytes = 8 + count * 4;
    require(
        bytes <= 68
            && at
                .checked_add(bytes)
                .is_some_and(|end| end <= start + length),
        "token SID bound rejected",
    )?;
    // SAFETY: the complete SID range was checked against initialized token output.
    require(
        unsafe { IsValidSid(pointer.cast_mut().cast()) } != 0,
        "invalid token SID",
    )?;
    // SAFETY: the validated byte range remains retained by storage through copying.
    Ok(unsafe { std::slice::from_raw_parts(pointer, bytes) }.to_vec())
}

pub(super) fn same_sid(left: &[u8], right: &[u8]) -> bool {
    // SAFETY: both values come only from bounded validated kernel SID copies.
    unsafe {
        EqualSid(
            left.as_ptr().cast_mut().cast(),
            right.as_ptr().cast_mut().cast(),
        ) != 0
    }
}

pub(super) fn token(process: HANDLE) -> Result<Token, WindowsProcessError> {
    let mut raw = null_mut();
    // SAFETY: exact retained process and writable token output; query authority only.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } == 0 {
        return Err(last_error("OpenProcessToken(browser context)"));
    }
    let guard = OwnedHandle::new(raw, "OpenProcessToken(browser context)")?;
    token_handle(guard.raw())
}

fn token_handle(token: HANDLE) -> Result<Token, WindowsProcessError> {
    let appcontainer = integer(token, TokenIsAppContainer)?;
    require(appcontainer <= 1, "invalid AppContainer token flag")?;
    let appcontainer = appcontainer == 1;
    let integrity = sid(token, TokenIntegrityLevel)?;
    require(
        integrity.len() == 12 && integrity[..8] == [1, 1, 0, 0, 0, 0, 0, 16],
        "token integrity SID is not a mandatory label",
    )?;
    let offset = integrity.len() - 4;
    let (statistics, length) = token_data(token, TokenStatistics)?;
    require(length >= 16, "token statistics truncated")?;
    // SAFETY: the AuthenticationId LUID occupies bytes 8..16 of TOKEN_STATISTICS.
    let authentication = unsafe {
        std::ptr::read_unaligned(statistics.as_ptr().cast::<u8>().add(8).cast::<[u32; 2]>())
    };
    Ok(Token {
        user: sid(token, TokenUser)?,
        session: integer(token, TokenSessionId)?,
        authentication,
        elevated: elevation(token)?,
        appcontainer,
        integrity: u32::from_le_bytes(
            integrity[offset..]
                .try_into()
                .map_err(|_| WindowsProcessError::Invalid("integrity size".into()))?,
        ),
        capabilities: capabilities(token)?,
        package: if appcontainer {
            Some(sid(token, TokenAppContainerSid)?)
        } else {
            None
        },
    })
}

/// Attenuates only this fresh context's own browser primary when it is elevated.
/// Core and the trusted creator retain their original tokens and cleanup authority.
pub(super) fn restricted_browser_primary() -> Result<Option<OwnedHandle>, WindowsProcessError> {
    // SAFETY: this fixed trusted context observes only its own process token.
    let current = token(unsafe { GetCurrentProcess() })?;
    if !current.elevated {
        return Ok(None);
    }
    let mut raw = null_mut();
    // CreateRestrictedToken preserves input handle rights; CreateProcessAsUser needs
    // precisely QUERY | DUPLICATE | ASSIGN_PRIMARY on the returned primary token.
    // SAFETY: opens only this process's own primary token with a writable output.
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY,
            &mut raw,
        )
    } == 0
    {
        return Err(last_error("OpenProcessToken(browser primary)"));
    }
    let source = OwnedHandle::new(raw, "OpenProcessToken(browser primary)")?;
    let mut raw = null_mut();
    // SAFETY: retains the own primary token, supplies no foreign SID/privilege lists,
    // and requests only LUA attenuation and privilege removal, never SANDBOX_INERT.
    if unsafe {
        CreateRestrictedToken(
            source.raw(),
            LUA_TOKEN | DISABLE_MAX_PRIVILEGE,
            0,
            null(),
            0,
            null(),
            0,
            null(),
            &mut raw,
        )
    } == 0
    {
        return Err(last_error("CreateRestrictedToken(browser primary)"));
    }
    let restricted = OwnedHandle::new(raw, "CreateRestrictedToken(browser primary)")?;
    let actual = token_handle(restricted.raw())?;
    require(
        same_sid(&actual.user, &current.user),
        "restricted browser primary user changed",
    )?;
    require(
        actual.session == current.session,
        "restricted browser primary session changed",
    )?;
    require(
        actual.authentication == current.authentication,
        "restricted browser primary authentication changed",
    )?;
    require(
        !actual.appcontainer,
        "restricted browser primary is an AppContainer",
    )?;
    require(
        !actual.elevated,
        "restricted browser primary remains elevated",
    )?;
    Ok(Some(restricted))
}

fn elevation(token: HANDLE) -> Result<bool, WindowsProcessError> {
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    // SAFETY: TokenElevation has this fixed documented output type. A null sizing
    // probe returns ERROR_BAD_LENGTH on supported hosts and is deliberately unused.
    if unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut elevation).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(last_error("GetTokenInformation(context elevation)"));
    }
    require(
        returned == size_of::<TOKEN_ELEVATION>() as u32 && elevation.TokenIsElevated <= 1,
        "token elevation output rejected",
    )?;
    Ok(elevation.TokenIsElevated == 1)
}

fn capabilities(token: HANDLE) -> Result<u32, WindowsProcessError> {
    let (storage, length) = token_data(token, TokenCapabilities)?;
    let offset = std::mem::offset_of!(TOKEN_GROUPS, Groups);
    require(length >= offset, "truncated token capabilities")?;
    // SAFETY: a complete TOKEN_GROUPS header was returned in aligned storage.
    let count = unsafe { std::ptr::read_unaligned(storage.as_ptr().cast::<u32>()) };
    require(
        (count as usize)
            .checked_mul(size_of::<SID_AND_ATTRIBUTES>())
            .and_then(|bytes| offset.checked_add(bytes))
            .is_some_and(|bytes| bytes <= length),
        "capability array exceeds token buffer",
    )?;
    Ok(count)
}

pub(super) fn ordinary(
    actual: &Token,
    core: &Token,
    fresh: bool,
) -> Result<(), WindowsProcessError> {
    require(
        same_sid(&actual.user, &core.user)
            && actual.session == core.session
            && !actual.appcontainer
            && actual.elevated == core.elevated
            && actual.integrity == core.integrity
            && if fresh {
                actual.authentication != core.authentication
            } else {
                actual.authentication == core.authentication
            },
        "trusted helper token differs from its Core context",
    )
}

pub(super) fn browser(
    actual: &Token,
    core: &Token,
    context: &Token,
    package: *mut std::ffi::c_void,
) -> Result<(), WindowsProcessError> {
    let package_matches = actual.package.as_ref().is_some_and(|actual| {
        // SAFETY: both SIDs are retained validated kernel/ConvertStringSidToSid output.
        unsafe { EqualSid(actual.as_ptr().cast_mut().cast(), package) != 0 }
    });
    let admitted = same_sid(&actual.user, &core.user)
        && actual.session == context.session
        && actual.authentication == context.authentication
        && actual.appcontainer
        && !actual.elevated
        && actual.integrity == 4096
        && actual.capabilities == 0
        && package_matches;
    if !admitted {
        // One bit per original admitted clause, in the same order. No SID/LUID is recorded.
        let mismatch = u16::from(!same_sid(&actual.user, &core.user))
            | (u16::from(actual.session != context.session) << 1)
            | (u16::from(actual.authentication != context.authentication) << 2)
            | (u16::from(!actual.appcontainer) << 3)
            | (u16::from(actual.elevated) << 4)
            | (u16::from(actual.integrity != 4096) << 5)
            | (u16::from(actual.capabilities != 0) << 6)
            | (u16::from(!package_matches) << 7);
        super::diagnostics::record_browser(mismatch);
    }
    #[cfg(debug_assertions)]
    if !admitted {
        eprintln!(
            "native Windows browser context token rejection same_user_core={} same_user_context={} same_session_core={} same_session_context={} appcontainer={} elevated={} integrity_rid={} capabilities={} authentication_equals_core={} authentication_equals_context={} context_authentication_fresh_core={} package_matches={}",
            same_sid(&actual.user, &core.user),
            same_sid(&actual.user, &context.user),
            actual.session == core.session,
            actual.session == context.session,
            actual.appcontainer,
            actual.elevated,
            actual.integrity,
            actual.capabilities,
            actual.authentication == core.authentication,
            actual.authentication == context.authentication,
            context.authentication != core.authentication,
            package_matches,
        );
    }
    require(
        admitted,
        "suspended browser token differs from exact fresh AppContainer context",
    )
}

pub(super) fn sid_text(sid: &[u8]) -> Result<String, WindowsProcessError> {
    let mut text = null_mut();
    // SAFETY: SID is a validated retained copy; Windows allocates text with LocalAlloc.
    if unsafe { ConvertSidToStringSidW(sid.as_ptr().cast_mut().cast(), &mut text) } == 0 {
        return Err(last_error("ConvertSidToStringSidW(context owner)"));
    }
    let mut length = 0;
    // SAFETY: the successful API supplies terminated SID text; numeric SID syntax is bounded.
    while length < 184 && unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    let result = if length < 184 {
        // SAFETY: the measured terminated range belongs to the retained allocation.
        String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
            .map_err(|_| WindowsProcessError::Invalid("owner SID encoding".into()))
    } else {
        Err(WindowsProcessError::Invalid("owner SID text bound".into()))
    };
    // SAFETY: this exact pointer was allocated by ConvertSidToStringSidW.
    unsafe {
        LocalFree(text.cast());
    }
    result
}

pub(super) fn birth(process: HANDLE) -> Result<u64, WindowsProcessError> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: retained process and correctly sized writable FILETIME outputs.
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(last_error("GetProcessTimes(browser context)"));
    }
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

pub(super) fn image(process: HANDLE) -> Result<BoundPath, WindowsProcessError> {
    let mut buffer = vec![0_u16; 32_768];
    let mut length = 32_768;
    // SAFETY: retained process and bounded writable image-path storage.
    if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err(last_error("QueryFullProcessImageNameW(browser context)"));
    }
    require(length > 0 && length < 32_768, "process image bound")?;
    let path = std::path::PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
    native(BoundPath::open_immutable_file(&path))
}

pub(super) fn verify_process(
    process: &OwnedHandle,
    pid: u32,
    created: u64,
    expected: FileIdentity,
) -> Result<(), WindowsProcessError> {
    // SAFETY: observation of the exact retained process, not a PID mutation lookup.
    require(
        unsafe { GetProcessId(process.raw()) } == pid
            && birth(process.raw())? == created
            && image(process.raw())?.identity() == expected,
        "retained helper birth/image identity changed",
    )
}

pub(super) fn duplicate(
    source: HANDLE,
    target_process: HANDLE,
    rights: u32,
    inherit: bool,
) -> Result<u64, WindowsProcessError> {
    let mut remote = null_mut();
    // SAFETY: retained source/target process; duplicate is reduced, explicitly inherited or not.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            target_process,
            &mut remote,
            rights,
            i32::from(inherit),
            0,
        )
    } == 0
    {
        return Err(last_error("DuplicateHandle(browser context target)"));
    }
    Ok(remote as usize as u64)
}

pub(super) fn pull(
    source_process: &OwnedHandle,
    remote: u64,
    rights: u32,
) -> Result<OwnedHandle, WindowsProcessError> {
    let remote = usize::try_from(remote)
        .map_err(|_| WindowsProcessError::Invalid("remote process reference width".into()))?;
    require(
        remote != 0 && remote <= isize::MAX as usize,
        "invalid or pseudo remote process reference",
    )?;
    let mut local = null_mut();
    // SAFETY: only the exact authenticated launcher's retained process supplies this reference.
    if unsafe {
        DuplicateHandle(
            source_process.raw(),
            remote as HANDLE,
            GetCurrentProcess(),
            &mut local,
            rights,
            0,
            0,
        )
    } == 0
    {
        return Err(last_error("DuplicateHandle(pull exact context)"));
    }
    OwnedHandle::new(local, "DuplicateHandle(pulled context)")
}

pub(super) fn resume(thread: &OwnedHandle) -> Result<(), WindowsProcessError> {
    // SAFETY: exact retained primary thread from suspended creation, resumed once.
    require(
        unsafe { ResumeThread(thread.raw()) } == 1,
        "primary thread suspension count differs",
    )
}

#[cfg(test)]
mod tests;
