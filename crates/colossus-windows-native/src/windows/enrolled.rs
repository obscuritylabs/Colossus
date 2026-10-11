//! A parent-enrolled directory handle is the child's filesystem authority root.
//! The supervisor retains/validates ancestry; the AppContainer cannot open C:\.
use super::*;
use std::sync::Arc;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_NAME_OPENED, GetFinalPathNameByHandleW, VOLUME_NAME_NONE,
};

fn volume_path(path: &Path) -> Result<PathBuf, WindowsNativeError> {
    let mut parts = path.components();
    match parts.next() {
        Some(std::path::Component::Prefix(prefix))
            if matches!(
                prefix.kind(),
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
            ) => {}
        _ => return Err(WindowsNativeError::InvalidInput),
    }
    Ok(parts.as_path().to_owned())
}

pub(super) struct Root {
    file: File,
    identity: FileIdentity,
    path: PathBuf,
}

fn handle_path(file: &File) -> Result<PathBuf, WindowsNativeError> {
    let mut output = vec![0u16; 32768];
    // SAFETY: retained file and bounded writable UTF-16 output. Query the
    // opened volume-relative path without opening inaccessible ambient volumes.
    // The parent supplied the canonical path and exact kernel identity; every
    // in-scope component is separately opened without following reparse points.
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle().cast(),
            output.as_mut_ptr(),
            output.len() as u32,
            FILE_NAME_OPENED | VOLUME_NAME_NONE,
        )
    };
    if length == 0 || length as usize >= output.len() {
        return Err(last_error("query enrolled directory handle path"));
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &output[..length as usize],
    )))
}

impl Root {
    fn validate(&self) -> Result<(), WindowsNativeError> {
        let information = file_information(&self.file)?;
        if information.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            != FILE_ATTRIBUTE_DIRECTORY
            || file_identity(&self.file)? != self.identity
            || handle_path(&self.file)? != volume_path(&self.path)?
        {
            return Err(WindowsNativeError::IdentityChanged);
        }
        validate_private_appcontainer_dacl(&self.file)
    }
}

pub(crate) fn bind(
    file: File,
    path: &Path,
    identity: FileIdentity,
) -> Result<BoundPathInner, WindowsNativeError> {
    let principal = AppContainerPrincipal::of_current()?;
    principal.validate_low_integrity()?;
    if !path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(WindowsNativeError::InvalidInput);
    }
    let root = Arc::new(Root {
        file,
        identity,
        path: path.to_owned(),
    });
    root.validate()?;
    let opened = root
        .file
        .try_clone()
        .map_err(|source| WindowsNativeError::Io {
            operation: "retain enrolled profile handle",
            source,
        })?;
    Ok(BoundPathInner {
        file: opened,
        canonical_path: path.to_owned(),
        identity,
        ancestors: Vec::new(),
        kind: BoundKind::Directory,
        immutable: false,
        appcontainer: true,
        appcontainer_scope: Some(root),
    })
}

pub(super) fn open(
    root: Arc<Root>,
    path: &Path,
    kind: BoundKind,
    access: u32,
    immutable: bool,
) -> Result<BoundPathInner, WindowsNativeError> {
    root.validate()?;
    let relative = path
        .strip_prefix(&root.path)
        .map_err(|_| WindowsNativeError::InvalidInput)?;
    let components = relative.components().collect::<Vec<_>>();
    if components.iter().any(|component| !matches!(component, std::path::Component::Normal(name) if !name.is_empty() && !name.encode_wide().any(|c| matches!(c, 0 | 58)))) {
        return Err(WindowsNativeError::InvalidInput);
    }
    let principal = AppContainerPrincipal::of_current()?;
    let current_root = open_exact(&root.path, BoundKind::Directory, GENERIC_READ, true, true)?;
    if file_identity(&current_root)? != root.identity {
        return Err(WindowsNativeError::IdentityChanged);
    }
    let mut ancestors = Vec::new();
    let mut candidate = root.path.clone();
    if !components.is_empty() {
        ancestors.push(current_root);
    }
    // Reopen only inside the parent-enrolled root. Every descendant component
    // is retained and checked without following reparses; no ambient root opens.
    for component in &components[..components.len().saturating_sub(1)] {
        candidate.push(component);
        let ancestor = open_exact(&candidate, BoundKind::Directory, GENERIC_READ, true, true)?;
        ancestors.push(ancestor);
    }
    if let Some(leaf) = components.last() {
        candidate.push(leaf);
    }
    let file = open_exact(
        &candidate,
        kind,
        access,
        immutable || matches!(kind, BoundKind::Directory),
        true,
    )?;
    validate_owner_dacl(
        &file,
        if matches!(kind, BoundKind::File) {
            DaclValidation::PrivateAppContainerFile(principal.package_sid())
        } else {
            DaclValidation::PrivateAppContainer(principal.package_sid())
        },
    )?;
    if handle_path(&file)? != volume_path(&candidate)? {
        return Err(WindowsNativeError::IdentityChanged);
    }
    root.validate()?;
    let identity = file_identity(&file)?;
    Ok(BoundPathInner {
        file,
        canonical_path: candidate,
        identity,
        ancestors,
        kind,
        immutable,
        appcontainer: true,
        appcontainer_scope: Some(root),
    })
}

pub(crate) fn reopen(
    bound: &BoundPathInner,
    path: &Path,
    kind: BoundKind,
    access: u32,
) -> Result<BoundPathInner, WindowsNativeError> {
    match &bound.appcontainer_scope {
        Some(scope) => open(Arc::clone(scope), path, kind, access, bound.immutable),
        None => open_bound_with_mode(path, kind, access, bound.immutable, bound.appcontainer),
    }
}
