use crate::{FileIdentity, WindowsNativeError};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    mem::{size_of, zeroed},
    os::windows::{
        ffi::{OsStrExt as _, OsStringExt as _},
        fs::OpenOptionsExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle, RawHandle},
        process::CommandExt as _,
    },
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, GENERIC_READ, INVALID_HANDLE_VALUE, LocalFree, NO_ERROR},
    Security::{
        ACCESS_ALLOWED_ACE, ACE_FLAGS, ACE_HEADER,
        Authorization::{
            EXPLICIT_ACCESS_W, GetSecurityInfo, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS,
            SetEntriesInAclW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
        },
        CreateWellKnownSid,
        Credentials::{
            CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_DO_NOT_PERSIST,
            CREDUI_FLAGS_EXCLUDE_CERTIFICATES, CREDUI_FLAGS_GENERIC_CREDENTIALS,
            CREDUI_FLAGS_KEEP_USERNAME, CREDUI_INFOW, CredUIPromptForCredentialsW,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation,
        InitializeSecurityDescriptor, IsValidSid, NO_INHERITANCE, OWNER_SECURITY_INFORMATION, PSID,
        SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SECURITY_MAX_SID_SIZE,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT, SetSecurityDescriptorControl,
        SetSecurityDescriptorDacl, SetSecurityDescriptorOwner, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WinBuiltinAdministratorsSid, WinLocalSystemSid,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ALL_ACCESS,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_WRITE, FILE_ID_INFO,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo,
        GetFileInformationByHandle, GetFileInformationByHandleEx, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, READ_CONTROL,
    },
    System::{
        Console::{STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle},
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        SystemServices::{
            ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE, SECURITY_DESCRIPTOR_REVISION,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, GetCurrentProcess, OpenProcessToken, OpenThread,
            QueryFullProcessImageNameW, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};
use zeroize::Zeroizing;

pub(super) fn configure_suspended_process(command: &mut std::process::Command) {
    command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
}

pub(super) fn create_private_directory(path: &Path) -> Result<(), WindowsNativeError> {
    let encoded = nul_terminated_path(path)?;
    let parent_path = path.parent().ok_or(WindowsNativeError::InvalidInput)?;
    let parent = open_bound(parent_path, BoundKind::Directory)?;

    with_private_security_attributes(
        PRIVATE_DIRECTORY_LABELS,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        |attributes| {
            // SAFETY: the path and security descriptor are NUL-terminated/live for the call.
            if unsafe { CreateDirectoryW(encoded.as_ptr(), attributes) } == 0 {
                return Err(last_error("create private directory"));
            }
            Ok(())
        },
    )?;

    let created = match open_bound(path, BoundKind::Directory) {
        Ok(created) => created,
        Err(error) => {
            let _ = fs::remove_dir(path);
            return Err(error);
        }
    };
    if let Err(error) = created
        .validate_private_owner_dacl()
        .and_then(|()| created.revalidate())
        .and_then(|()| parent.revalidate())
    {
        let _ = fs::remove_dir(path);
        return Err(error);
    }
    Ok(())
}

pub(super) fn create_private_file(path: &Path, contents: &[u8]) -> Result<(), WindowsNativeError> {
    let encoded = nul_terminated_path(path)?;
    let parent_path = path.parent().ok_or(WindowsNativeError::InvalidInput)?;
    let parent = open_bound(parent_path, BoundKind::Directory)?;

    let handle =
        with_private_security_attributes(PRIVATE_FILE_LABELS, NO_INHERITANCE, |attributes| {
            // SAFETY: the path and security descriptor are NUL-terminated/live for the call.
            // CREATE_NEW plus FILE_FLAG_OPEN_REPARSE_POINT never follows an existing name.
            let handle = unsafe {
                CreateFileW(
                    encoded.as_ptr(),
                    FILE_GENERIC_WRITE,
                    0,
                    attributes,
                    CREATE_NEW,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                    null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE || handle.is_null() {
                return Err(last_error("create private file"));
            }
            // SAFETY: CreateFileW returned one newly owned valid handle.
            Ok(unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
        })?;

    let mut file = File::from(handle);
    if let Err(source) =
        std::io::Write::write_all(&mut file, contents).and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(WindowsNativeError::Io {
            operation: "write private file",
            source,
        });
    }
    drop(file);

    let created = match open_bound(path, BoundKind::File) {
        Ok(created) => created,
        Err(error) => {
            let _ = fs::remove_file(path);
            return Err(error);
        }
    };
    if let Err(error) = created
        .validate_private_owner_dacl()
        .and_then(|()| created.revalidate())
        .and_then(|()| parent.revalidate())
    {
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

/// Stable operation labels reported when private security construction fails.
#[derive(Clone, Copy)]
struct PrivateSecurityLabels {
    dacl: &'static str,
    descriptor: &'static str,
}

const PRIVATE_DIRECTORY_LABELS: PrivateSecurityLabels = PrivateSecurityLabels {
    dacl: "build private directory DACL",
    descriptor: "build private directory security descriptor",
};

const PRIVATE_FILE_LABELS: PrivateSecurityLabels = PrivateSecurityLabels {
    dacl: "build private file DACL",
    descriptor: "build private file security descriptor",
};

/// Build one protected owner-only security descriptor and lend it to `action`.
///
/// The DACL grants the current user, `LocalSystem`, and `BUILTIN\Administrators`
/// only, and `SE_DACL_PROTECTED` blocks every inherited parent entry.
fn with_private_security_attributes<T>(
    labels: PrivateSecurityLabels,
    inheritance: ACE_FLAGS,
    action: impl FnOnce(&SECURITY_ATTRIBUTES) -> Result<T, WindowsNativeError>,
) -> Result<T, WindowsNativeError> {
    let current_user_storage = current_user_sid()?;
    let local_system_storage = well_known_sid(WinLocalSystemSid)?;
    let administrators_storage = well_known_sid(WinBuiltinAdministratorsSid)?;
    let current_user = current_user_storage.as_ptr().cast_mut().cast();
    let local_system = local_system_storage.as_ptr().cast_mut().cast();
    let administrators = administrators_storage.as_ptr().cast_mut().cast();
    let mut entries = [
        private_access_entry(current_user, inheritance),
        private_access_entry(local_system, inheritance),
        private_access_entry(administrators, inheritance),
    ];
    let mut acl = null_mut();
    // SAFETY: all three trustee SIDs remain valid until the allocated ACL is built.
    let result = unsafe {
        SetEntriesInAclW(
            u32::try_from(entries.len()).expect("private ACL entry count fits u32"),
            entries.as_mut_ptr(),
            null(),
            &raw mut acl,
        )
    };
    if result != NO_ERROR || acl.is_null() {
        return Err(WindowsNativeError::Io {
            operation: labels.dacl,
            source: std::io::Error::from_raw_os_error(i32::try_from(result).unwrap_or(i32::MAX)),
        });
    }
    let _acl = LocalAcl(acl);

    let mut descriptor = SECURITY_DESCRIPTOR::default();
    // SAFETY: descriptor is writable and all borrowed ACL/SID storage outlives the call.
    if !unsafe {
        InitializeSecurityDescriptor((&raw mut descriptor).cast(), SECURITY_DESCRIPTOR_REVISION)
            != 0
            && SetSecurityDescriptorOwner((&raw mut descriptor).cast(), current_user, 0) != 0
            && SetSecurityDescriptorDacl((&raw mut descriptor).cast(), 1, acl, 0) != 0
            && SetSecurityDescriptorControl(
                (&raw mut descriptor).cast(),
                SE_DACL_PROTECTED,
                SE_DACL_PROTECTED,
            ) != 0
    } {
        return Err(last_error(labels.descriptor));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .expect("security attributes size fits u32"),
        lpSecurityDescriptor: (&raw mut descriptor).cast(),
        bInheritHandle: 0,
    };
    action(&attributes)
}

pub(super) fn replace_private_file(
    source_path: &Path,
    destination_path: &Path,
) -> Result<(), WindowsNativeError> {
    if source_path == destination_path
        || source_path.parent().is_none()
        || source_path.parent() != destination_path.parent()
    {
        return Err(WindowsNativeError::InvalidInput);
    }
    let parent_path = source_path
        .parent()
        .ok_or(WindowsNativeError::InvalidInput)?;
    let parent = open_bound(parent_path, BoundKind::Directory)?;
    parent.validate_ancestor_namespace_authority()?;
    parent.validate_private_owner_dacl()?;
    parent.revalidate()?;
    let source = open_bound(source_path, BoundKind::File)?;
    source.validate_ancestor_namespace_authority()?;
    source.validate_private_owner_dacl()?;
    source.revalidate()?;
    if destination_path.exists() {
        let destination = open_bound(destination_path, BoundKind::File)?;
        destination.validate_ancestor_namespace_authority()?;
        destination.validate_private_owner_dacl()?;
        destination.revalidate()?;
    }

    let source_encoded = nul_terminated_path(source_path)?;
    let destination_encoded = nul_terminated_path(destination_path)?;
    // SAFETY: both paths are NUL-terminated and remain live for the call. The retained
    // private parent and source handles make the post-operation identity check meaningful.
    if unsafe {
        MoveFileExW(
            source_encoded.as_ptr(),
            destination_encoded.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(last_error("replace private file"));
    }

    let committed = open_bound(destination_path, BoundKind::File)?;
    if committed.identity != source.identity {
        return Err(WindowsNativeError::IdentityChanged);
    }
    committed
        .validate_ancestor_namespace_authority()
        .and_then(|()| committed.validate_private_owner_dacl())
        .and_then(|()| committed.revalidate())
        .and_then(|()| parent.revalidate())
}

fn nul_terminated_path(path: &Path) -> Result<Vec<u16>, WindowsNativeError> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(WindowsNativeError::InvalidInput);
    }
    let mut encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(WindowsNativeError::InvalidInput);
    }
    encoded.push(0);
    Ok(encoded)
}

fn private_access_entry(sid: PSID, inheritance: ACE_FLAGS) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: inheritance,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.cast(),
        },
    }
}

#[derive(Clone, Copy)]
pub(super) enum BoundKind {
    Directory,
    File,
}

pub(super) struct BoundPathInner {
    pub(super) file: File,
    pub(super) canonical_path: PathBuf,
    pub(super) identity: FileIdentity,
    ancestors: Vec<File>,
    kind: BoundKind,
}

pub(super) struct KillOnCloseJob {
    handle: OwnedHandle,
}

impl KillOnCloseJob {
    pub(super) fn assign_and_verify(
        process: RawHandle,
        process_id: u32,
        expected_image: FileIdentity,
    ) -> Result<Self, WindowsNativeError> {
        if process_id == 0 {
            return Err(WindowsNativeError::InvalidInput);
        }
        // SAFETY: null security attributes and name create one private unnamed job.
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return Err(last_error("create kill-on-close job"));
        }
        // SAFETY: CreateJobObjectW returned one newly owned non-null handle.
        let job = Self {
            handle: unsafe { OwnedHandle::from_raw_handle(handle.cast()) },
        };
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the buffer matches JobObjectExtendedLimitInformation exactly.
        if unsafe {
            SetInformationJobObject(
                job.handle.as_raw_handle().cast(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .expect("job limits size fits u32"),
            )
        } == 0
        {
            return Err(last_error("configure kill-on-close job"));
        }
        // SAFETY: both borrowed handles remain valid for the call.
        if unsafe { AssignProcessToJobObject(job.handle.as_raw_handle().cast(), process.cast()) }
            == 0
        {
            return Err(last_error("assign process to kill-on-close job"));
        }
        verify_process_image(process, expected_image)?;
        Ok(job)
    }

    pub(super) fn assign_verify_and_resume(
        process: RawHandle,
        process_id: u32,
        expected_image: FileIdentity,
    ) -> Result<Self, WindowsNativeError> {
        let job = Self::assign_and_verify(process, process_id, expected_image)?;
        resume_suspended_process(process_id)?;
        Ok(job)
    }

    pub(super) fn terminate(&self) -> Result<(), WindowsNativeError> {
        // SAFETY: the Job Object remains owned for the call.
        if unsafe { TerminateJobObject(self.handle.as_raw_handle().cast(), 1) } == 0 {
            Err(last_error("terminate process job"))
        } else {
            Ok(())
        }
    }
}

fn verify_process_image(
    process: RawHandle,
    expected_image: FileIdentity,
) -> Result<(), WindowsNativeError> {
    let mut buffer = vec![0_u16; 32_768];
    let mut length = u32::try_from(buffer.len()).expect("Windows path bound fits u32");
    // SAFETY: the output buffer is writable for the advertised length and the process
    // handle remains borrowed for this query.
    if unsafe {
        QueryFullProcessImageNameW(process.cast(), 0, buffer.as_mut_ptr(), &raw mut length)
    } == 0
    {
        return Err(last_error("query suspended process image"));
    }
    let length = usize::try_from(length).map_err(|_| WindowsNativeError::IdentityChanged)?;
    if length == 0 || length > buffer.len() {
        return Err(WindowsNativeError::IdentityChanged);
    }
    let image = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
    let binding = open_bound(&image, BoundKind::File)?;
    if binding.identity != expected_image {
        return Err(WindowsNativeError::IdentityChanged);
    }
    binding.revalidate()
}

pub(super) fn resume_suspended_process(process_id: u32) -> Result<(), WindowsNativeError> {
    // SAFETY: snapshot creation has no borrowed pointer arguments.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(last_error("snapshot suspended process threads"));
    }
    // SAFETY: the valid snapshot handle transfers to OwnedHandle.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.cast()) };
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = u32::try_from(size_of::<THREADENTRY32>()).expect("thread entry size fits u32");
    let mut thread_id = None;
    // SAFETY: entry is initialized with the documented structure size.
    let mut available =
        unsafe { Thread32First(snapshot.as_raw_handle().cast(), &raw mut entry) } != 0;
    while available {
        if entry.th32OwnerProcessID == process_id {
            if thread_id.replace(entry.th32ThreadID).is_some() {
                return Err(WindowsNativeError::IdentityChanged);
            }
        }
        // SAFETY: the snapshot and writable entry remain valid for iteration.
        available = unsafe { Thread32Next(snapshot.as_raw_handle().cast(), &raw mut entry) } != 0;
    }
    let thread_id = thread_id.ok_or(WindowsNativeError::IdentityChanged)?;
    // SAFETY: the selected thread ID belongs to the still-suspended child.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
    if thread.is_null() {
        return Err(last_error("open suspended process thread"));
    }
    // SAFETY: OpenThread returned one newly owned non-null handle.
    let thread = unsafe { OwnedHandle::from_raw_handle(thread.cast()) };
    // SAFETY: the thread handle includes THREAD_SUSPEND_RESUME.
    if unsafe { ResumeThread(thread.as_raw_handle().cast()) } != 1 {
        return Err(WindowsNativeError::IdentityChanged);
    }
    Ok(())
}

pub(super) fn validate_named_pipe_client(
    pipe: RawHandle,
    expected_process_id: u32,
) -> Result<(), WindowsNativeError> {
    validate_named_pipe_peer(pipe, expected_process_id, true)
}

pub(super) fn validate_named_pipe_server(
    pipe: RawHandle,
    expected_process_id: u32,
) -> Result<(), WindowsNativeError> {
    validate_named_pipe_peer(pipe, expected_process_id, false)
}

fn validate_named_pipe_peer(
    pipe: RawHandle,
    expected_process_id: u32,
    client: bool,
) -> Result<(), WindowsNativeError> {
    if expected_process_id == 0 {
        return Err(WindowsNativeError::InvalidInput);
    }
    let mut process_id = 0;
    // SAFETY: the output PID pointer is valid and the pipe handle remains borrowed.
    let result = unsafe {
        if client {
            GetNamedPipeClientProcessId(pipe.cast(), &raw mut process_id)
        } else {
            GetNamedPipeServerProcessId(pipe.cast(), &raw mut process_id)
        }
    };
    if result == 0 {
        return Err(last_error("validate named-pipe peer"));
    }
    if process_id != expected_process_id {
        return Err(WindowsNativeError::IdentityChanged);
    }
    Ok(())
}

pub(super) fn install_bootstrap_pipe_as_standard_io(
    pipe: RawHandle,
) -> Result<(), WindowsNativeError> {
    // SAFETY: SetStdHandle borrows but does not close the caller-owned pipe handle.
    if unsafe {
        SetStdHandle(STD_INPUT_HANDLE, pipe.cast()) != 0
            && SetStdHandle(STD_OUTPUT_HANDLE, pipe.cast()) != 0
    } {
        Ok(())
    } else {
        Err(last_error("install bootstrap named pipe"))
    }
}

impl BoundPathInner {
    #[cfg(test)]
    pub(super) fn retained_ancestor_count(&self) -> usize {
        self.ancestors.len()
    }

    pub(super) fn revalidate(&self) -> Result<(), WindowsNativeError> {
        let current = open_bound(&self.canonical_path, self.kind)?;
        if current.identity != self.identity || file_identity(&self.file)? != self.identity {
            return Err(WindowsNativeError::IdentityChanged);
        }
        if current.ancestors.len() != self.ancestors.len() {
            return Err(WindowsNativeError::IdentityChanged);
        }
        for (index, (actual, expected)) in current.ancestors.iter().zip(&self.ancestors).enumerate()
        {
            let unchanged = if index == 0 {
                root_file_identity(actual)? == root_file_identity(expected)?
            } else {
                file_identity(actual)? == file_identity(expected)?
            };
            if !unchanged {
                return Err(WindowsNativeError::IdentityChanged);
            }
        }
        Ok(())
    }

    pub(super) fn validate_private_owner_dacl(&self) -> Result<(), WindowsNativeError> {
        validate_private_owner_dacl(&self.file)
    }

    pub(super) fn validate_ancestor_namespace_authority(&self) -> Result<(), WindowsNativeError> {
        self.ancestors
            .iter()
            .try_for_each(validate_namespace_owner_dacl)
    }

    pub(super) fn validate_namespace_authority(&self) -> Result<(), WindowsNativeError> {
        self.validate_ancestor_namespace_authority()?;
        validate_namespace_owner_dacl(&self.file)
    }
}

pub(super) fn open_bound(
    path: &Path,
    kind: BoundKind,
) -> Result<BoundPathInner, WindowsNativeError> {
    open_bound_with_access(path, kind, false)
}

pub(super) fn open_bound_file_read_write(
    path: &Path,
) -> Result<BoundPathInner, WindowsNativeError> {
    open_bound_with_access(path, BoundKind::File, true)
}

fn open_bound_with_access(
    path: &Path,
    kind: BoundKind,
    writable_file: bool,
) -> Result<BoundPathInner, WindowsNativeError> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(WindowsNativeError::InvalidInput);
    }
    let mut ancestors = Vec::new();
    let mut candidate = PathBuf::new();
    let components = path.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        candidate.push(component.as_os_str());
        // A verbatim or UNC prefix can report itself as rooted before its RootDir
        // component is consumed. Open exactly the RootDir once so ancestor index zero
        // always retains the volume or share root, including during canonical reopens.
        if matches!(*component, std::path::Component::Prefix(_)) || !candidate.has_root() {
            continue;
        }
        let leaf = index + 1 == components.len();
        let component_kind = if leaf { kind } else { BoundKind::Directory };
        let opened = open_exact(&candidate, component_kind, leaf && writable_file)?;
        if leaf {
            let canonical_path =
                fs::canonicalize(path).map_err(|source| WindowsNativeError::Io {
                    operation: "canonicalize bound path",
                    source,
                })?;
            let identity = file_identity(&opened)?;
            return Ok(BoundPathInner {
                file: opened,
                canonical_path,
                identity,
                ancestors,
                kind,
            });
        }
        ancestors.push(opened);
    }
    Err(WindowsNativeError::InvalidInput)
}

fn open_exact(
    path: &Path,
    kind: BoundKind,
    writable_file: bool,
) -> Result<File, WindowsNativeError> {
    let mut options = OpenOptions::new();
    let data_access = if matches!(kind, BoundKind::File) {
        GENERIC_READ | if writable_file { FILE_GENERIC_WRITE } else { 0 }
    } else {
        0
    };
    options
        .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | data_access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT
                | if matches!(kind, BoundKind::Directory) {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        );
    let file = options
        .open(path)
        .map_err(|source| WindowsNativeError::Io {
            operation: "open path without following reparse points",
            source,
        })?;
    let information = file_information(&file)?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(WindowsNativeError::ReparsePoint);
    }
    let directory = information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if directory != matches!(kind, BoundKind::Directory) {
        return Err(WindowsNativeError::InvalidInput);
    }
    Ok(file)
}

struct LocalSecurityDescriptor(*mut core::ffi::c_void);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: GetSecurityInfo allocates the descriptor with LocalAlloc and
            // transfers exactly one LocalFree obligation to the caller.
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

struct LocalAcl(*mut windows_sys::Win32::Security::ACL);

impl Drop for LocalAcl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: SetEntriesInAclW allocates the ACL with LocalAlloc and transfers
            // exactly one LocalFree obligation to the caller.
            unsafe {
                LocalFree(self.0.cast());
            }
        }
    }
}

#[derive(Clone, Copy)]
enum DaclValidation {
    Private,
    NamespaceAuthority,
}

const NAMESPACE_MUTATION_RIGHTS: u32 = 0x0000_0040 // FILE_DELETE_CHILD
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000 // WRITE_OWNER
    | 0x1000_0000; // GENERIC_ALL
const INHERIT_ONLY_ACE_FLAG: u8 = 0x08;

fn validate_private_owner_dacl(file: &File) -> Result<(), WindowsNativeError> {
    validate_owner_dacl(file, DaclValidation::Private)
}

fn validate_namespace_owner_dacl(file: &File) -> Result<(), WindowsNativeError> {
    validate_owner_dacl(file, DaclValidation::NamespaceAuthority)
}

fn validate_owner_dacl(file: &File, validation: DaclValidation) -> Result<(), WindowsNativeError> {
    let current_user_storage = current_user_sid()?;
    let current_user = current_user_storage.as_ptr().cast_mut().cast();
    let mut owner: PSID = null_mut();
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: the file handle includes READ_CONTROL; output pointers are valid and
    // the returned descriptor remains alive until all borrowed SIDs are inspected.
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            null_mut(),
            &raw mut dacl,
            null_mut(),
            &raw mut descriptor,
        )
    };
    if result != NO_ERROR {
        return Err(WindowsNativeError::Io {
            operation: "query owner and DACL",
            source: std::io::Error::from_raw_os_error(i32::try_from(result).unwrap_or(i32::MAX)),
        });
    }
    let _descriptor = LocalSecurityDescriptor(descriptor);
    let local_system = well_known_sid(WinLocalSystemSid)?;
    let administrators = well_known_sid(WinBuiltinAdministratorsSid)?;
    let trusted_installer = trusted_installer_sid();
    // Elevated Windows tokens can use BUILTIN\Administrators as the default owner
    // for newly inherited child objects. All accepted owners are already the only
    // principals permitted by the private DACL.
    if owner.is_null()
        || dacl.is_null()
        || unsafe { IsValidSid(owner) } == 0
        || !(sid_is_one_of(
            owner,
            current_user,
            local_system.as_ptr().cast_mut().cast(),
            administrators.as_ptr().cast_mut().cast(),
        ) || matches!(validation, DaclValidation::NamespaceAuthority)
            && sid_matches(owner, trusted_installer.as_ptr().cast_mut().cast()))
    {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    // SAFETY: dacl points inside the live security descriptor and therefore its
    // fixed header can be read for the descriptor lifetime.
    let ace_count = unsafe { (*dacl).AceCount };
    for index in 0..u32::from(ace_count) {
        let mut ace = null_mut();
        // SAFETY: the DACL is valid and GetAce bounds-checks the requested index.
        if unsafe { GetAce(dacl, index, &raw mut ace) } == 0 || ace.is_null() {
            return Err(last_error("inspect DACL entry"));
        }
        // SAFETY: a successful GetAce returns at least one ACE_HEADER.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if matches!(validation, DaclValidation::NamespaceAuthority)
            && header.AceFlags & INHERIT_ONLY_ACE_FLAG != 0
        {
            continue;
        }
        match u32::from(header.AceType) {
            ACCESS_ALLOWED_ACE_TYPE => {
                if usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
                    return Err(WindowsNativeError::UnsafePermissions);
                }
                // SAFETY: the header size was checked for ACCESS_ALLOWED_ACE and
                // SidStart is the documented start of its variable-size SID.
                let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
                let sid = std::ptr::addr_of!(allowed.SidStart).cast_mut().cast();
                let trusted = unsafe { IsValidSid(sid) } != 0
                    && (sid_is_one_of(
                        sid,
                        current_user,
                        local_system.as_ptr().cast_mut().cast(),
                        administrators.as_ptr().cast_mut().cast(),
                    ) || matches!(validation, DaclValidation::NamespaceAuthority)
                        && sid_matches(sid, trusted_installer.as_ptr().cast_mut().cast()));
                if !trusted
                    && (matches!(validation, DaclValidation::Private)
                        || allowed.Mask & NAMESPACE_MUTATION_RIGHTS != 0)
                {
                    return Err(WindowsNativeError::UnsafePermissions);
                }
            }
            ACCESS_DENIED_ACE_TYPE => {}
            _ => return Err(WindowsNativeError::UnsafePermissions),
        }
    }
    Ok(())
}

fn current_user_sid() -> Result<Box<[u8; SECURITY_MAX_SID_SIZE as usize]>, WindowsNativeError> {
    let mut token = null_mut();
    // SAFETY: GetCurrentProcess is always valid and token receives one owned handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(last_error("open process token"));
    }
    // SAFETY: ownership of the non-null token handle transfers to OwnedHandle.
    let token = unsafe { OwnedHandle::from_raw_handle(token.cast()) };
    let mut required = 0;
    // SAFETY: the first call intentionally queries the required bounded size.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            TokenUser,
            null_mut(),
            0,
            &raw mut required,
        );
    }
    if required < u32::try_from(size_of::<TOKEN_USER>()).expect("TOKEN_USER size fits u32")
        || required > 64 * 1024
    {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let mut buffer = vec![
        0_u8;
        usize::try_from(required)
            .map_err(|_| { WindowsNativeError::UnsafePermissions })?
    ];
    // SAFETY: the output buffer has exactly the queried size and the token remains open.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &raw mut required,
        )
    } == 0
    {
        return Err(last_error("query process token user"));
    }
    // SAFETY: GetTokenInformation wrote TOKEN_USER at the buffer start; unaligned
    // read avoids assuming Vec<u8> alignment. The SID itself is token-owned and
    // remains valid only while this function's OwnedHandle is alive.
    let user = unsafe { std::ptr::read_unaligned(buffer.as_ptr().cast::<TOKEN_USER>()) };
    if user.User.Sid.is_null() || unsafe { IsValidSid(user.User.Sid) } == 0 {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    // Copy into a stable, self-contained well-bounded buffer before the token closes.
    let length = unsafe { windows_sys::Win32::Security::GetLengthSid(user.User.Sid) };
    if length == 0 || length > SECURITY_MAX_SID_SIZE {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let mut copied = Box::new([0_u8; SECURITY_MAX_SID_SIZE as usize]);
    if unsafe {
        windows_sys::Win32::Security::CopySid(
            SECURITY_MAX_SID_SIZE,
            copied.as_mut_ptr().cast(),
            user.User.Sid,
        )
    } == 0
    {
        return Err(last_error("copy process token user SID"));
    }
    Ok(copied)
}

fn well_known_sid(
    kind: i32,
) -> Result<Box<[u8; SECURITY_MAX_SID_SIZE as usize]>, WindowsNativeError> {
    let mut sid = Box::new([0_u8; SECURITY_MAX_SID_SIZE as usize]);
    let mut length = SECURITY_MAX_SID_SIZE;
    // SAFETY: the destination has SECURITY_MAX_SID_SIZE bytes and the domain is null
    // for machine-independent built-in well-known SIDs.
    if unsafe { CreateWellKnownSid(kind, null_mut(), sid.as_mut_ptr().cast(), &raw mut length) }
        == 0
    {
        return Err(last_error("create well-known SID"));
    }
    Ok(sid)
}

fn trusted_installer_sid() -> [u8; 32] {
    // NT SERVICE\TrustedInstaller is the fixed service SID
    // S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464.
    let mut sid = [0_u8; 32];
    sid[0] = 1;
    sid[1] = 6;
    sid[7] = 5;
    for (index, authority) in [
        80_u32,
        956_008_885,
        3_418_522_649,
        1_831_038_044,
        1_853_292_631,
        2_271_478_464,
    ]
    .into_iter()
    .enumerate()
    {
        let start = 8 + index * 4;
        sid[start..start + 4].copy_from_slice(&authority.to_le_bytes());
    }
    sid
}

fn sid_matches(sid: PSID, candidate: PSID) -> bool {
    // SAFETY: callers supply SIDs validated by Windows or the fixed TrustedInstaller SID.
    unsafe { EqualSid(sid, candidate) != 0 }
}

fn sid_is_one_of(sid: PSID, first: PSID, second: PSID, third: PSID) -> bool {
    // SAFETY: every argument was validated or created by the Windows SID APIs and
    // remains alive for this comparison.
    sid_matches(sid, first) || sid_matches(sid, second) || sid_matches(sid, third)
}

fn file_information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, WindowsNativeError> {
    // SAFETY: the output points to an initialized structure and the borrowed File keeps
    // the HANDLE valid for the call. This legacy query is supported across the Windows
    // filesystems used by Colossus and still describes the retained no-follow handle.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let result = unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &raw mut info) };
    if result == 0 {
        Err(last_error("query file information"))
    } else {
        Ok(info)
    }
}

fn file_identity(file: &File) -> Result<FileIdentity, WindowsNativeError> {
    // SAFETY: the output points to an initialized fixed-size structure for the exact
    // information class and the borrowed File keeps the HANDLE valid for the call.
    let mut info: FILE_ID_INFO = unsafe { zeroed() };
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle().cast(),
            FileIdInfo,
            (&raw mut info).cast(),
            u32::try_from(size_of::<FILE_ID_INFO>()).expect("structure size fits u32"),
        )
    };
    if result == 0 {
        Err(last_error("query file identity"))
    } else {
        Ok(FileIdentity {
            volume_serial_number: info.VolumeSerialNumber,
            file_id: info.FileId.Identifier,
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct RootFileIdentity {
    volume_serial_number: u32,
    file_index: u64,
}

fn root_file_identity(file: &File) -> Result<RootFileIdentity, WindowsNativeError> {
    // Volume roots on hosted filesystems may not implement FileIdInfo, but the legacy
    // retained-handle query still supplies a stable volume serial and 64-bit file index.
    // Leaves and non-root ancestors continue to require the stronger 128-bit FileIdInfo.
    let info = file_information(file)?;
    Ok(RootFileIdentity {
        volume_serial_number: info.dwVolumeSerialNumber,
        file_index: u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow),
    })
}

pub(super) fn file_link_count(file: &File) -> Result<u64, WindowsNativeError> {
    let info = file_information(file)?;
    if info.nNumberOfLinks == 0 {
        return Err(WindowsNativeError::InvalidInput);
    }
    Ok(u64::from(info.nNumberOfLinks))
}

pub(super) fn prompt_secret(
    title: &str,
    message: &str,
    target: &str,
    maximum_chars: usize,
) -> Result<Zeroizing<String>, WindowsNativeError> {
    let title = wide(title);
    let message = wide(message);
    let target = wide(target);
    let mut username = wide("provider");
    username.resize(514, 0);
    let mut password = vec![0_u16; maximum_chars + 1];
    let mut save = 0;
    let info = CREDUI_INFOW {
        cbSize: u32::try_from(size_of::<CREDUI_INFOW>()).expect("structure size fits u32"),
        hwndParent: std::ptr::null_mut(),
        pszMessageText: message.as_ptr(),
        pszCaptionText: title.as_ptr(),
        hbmBanner: std::ptr::null_mut(),
    };
    // SAFETY: all strings are NUL terminated, output buffers have the advertised
    // lengths, persistence is disabled, and no authentication context is supplied.
    let result = unsafe {
        CredUIPromptForCredentialsW(
            &raw const info,
            target.as_ptr(),
            null(),
            0,
            username.as_mut_ptr(),
            u32::try_from(username.len()).expect("username bound fits u32"),
            password.as_mut_ptr(),
            u32::try_from(password.len()).expect("password bound fits u32"),
            &raw mut save,
            CREDUI_FLAGS_ALWAYS_SHOW_UI
                | CREDUI_FLAGS_DO_NOT_PERSIST
                | CREDUI_FLAGS_EXCLUDE_CERTIFICATES
                | CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_KEEP_USERNAME,
        )
    };
    if result == ERROR_CANCELLED {
        password.fill(0);
        return Err(WindowsNativeError::Cancelled);
    }
    if result != NO_ERROR {
        password.fill(0);
        return Err(last_error("show credential prompt"));
    }
    let end = password
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(password.len());
    let decoded = String::from_utf16(&password[..end]).map_err(|_| {
        password.fill(0);
        WindowsNativeError::InvalidInput
    })?;
    password.fill(0);
    if decoded.is_empty() {
        return Err(WindowsNativeError::InvalidInput);
    }
    Ok(Zeroizing::new(decoded))
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn last_error(operation: &'static str) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation,
        source: std::io::Error::last_os_error(),
    }
}
