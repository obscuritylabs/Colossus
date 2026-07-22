//! Bounded extraction of the native Mach-O CodeDirectory identity.
//!
//! Security.framework identifies a running image by its CodeDirectory hash.  The
//! launcher derives that hash from the same private snapshot whose complete bytes
//! matched the signed-manifest SHA-256, rather than asking a path-based tool that can
//! race a bundle replacement.

use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
};

use sha2::{Digest as _, Sha256};

use crate::{SdkError, SdkResult};

const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;
const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;
const CSMAGIC_CODEDIRECTORY: u32 = 0xfade_0c02;
const CSSLOT_CODEDIRECTORY: u32 = 0;
const CSSLOT_ALTERNATE_CODEDIRECTORIES: u32 = 0x1000;
const CSSLOT_ALTERNATE_CODEDIRECTORY_LIMIT: u32 = 0x1005;
const CS_HASHTYPE_SHA256: u8 = 2;
const CS_HASHTYPE_SHA256_TRUNCATED: u8 = 3;
const MAX_FAT_ARCHITECTURES: usize = 32;
const MAX_LOAD_COMMAND_BYTES: usize = 4 * 1024 * 1024;
const MAX_CODE_SIGNATURE_BYTES: usize = 16 * 1024 * 1024;
// Darwin's linker may round the LC_CODE_SIGNATURE range up to its next
// 16-byte boundary even though the embedded superblob records its exact length.
const MAX_SUPERBLOB_PADDING_BYTES: usize = 15;

#[cfg(target_arch = "aarch64")]
const NATIVE_CPU_TYPE: u32 = 0x0100_000c;
#[cfg(target_arch = "x86_64")]
const NATIVE_CPU_TYPE: u32 = 0x0100_0007;

/// Exact 20-byte identity used by macOS dynamic code validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CodeDirectoryHash([u8; 20]);

impl CodeDirectoryHash {
    pub(super) fn requirement(self) -> String {
        format!("cdhash H\"{}\"", hex::encode(self.0))
    }
}

/// Opaque identity for one manifest-verified native macOS executable.
///
/// Obtain this only through [`crate::verify_macos_executable_identity`]. It can
/// then bind a kernel-suspended child before any credential-bearing channel is
/// released to that process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacosCodeIdentity(pub(super) CodeDirectoryHash);

pub(super) fn code_directory_hash(file: &mut File) -> SdkResult<CodeDirectoryHash> {
    let file_len = file
        .metadata()
        .map_err(|_| SdkError::IdentityMismatch)?
        .len();
    let (slice_offset, slice_len) = native_slice(file, file_len)?;
    let (signature_offset, signature_len) = code_signature_range(file, slice_offset, slice_len)?;
    let signature = read_bounded_range(
        file,
        signature_offset,
        signature_len,
        MAX_CODE_SIGNATURE_BYTES,
    )?;
    parse_superblob(&signature)
}

fn native_slice(file: &mut File, file_len: u64) -> SdkResult<(u64, u64)> {
    let header = read_bounded_range(file, 0, 8, 8)?;
    let magic_be = be_u32(&header, 0)?;
    if magic_be != FAT_MAGIC && magic_be != FAT_MAGIC_64 {
        return Ok((0, file_len));
    }
    let architectures =
        usize::try_from(be_u32(&header, 4)?).map_err(|_| SdkError::IdentityMismatch)?;
    if architectures == 0 || architectures > MAX_FAT_ARCHITECTURES {
        return Err(SdkError::IdentityMismatch);
    }
    let entry_len = if magic_be == FAT_MAGIC { 20 } else { 32 };
    let table_len = architectures
        .checked_mul(entry_len)
        .and_then(|length| length.checked_add(8))
        .ok_or(SdkError::IdentityMismatch)?;
    let table = read_bounded_range(file, 0, table_len as u64, 8 + 32 * MAX_FAT_ARCHITECTURES)?;
    for index in 0..architectures {
        let base = 8 + index * entry_len;
        if be_u32(&table, base)? != NATIVE_CPU_TYPE {
            continue;
        }
        let (offset, length) = if magic_be == FAT_MAGIC {
            (
                u64::from(be_u32(&table, base + 8)?),
                u64::from(be_u32(&table, base + 12)?),
            )
        } else {
            (be_u64(&table, base + 8)?, be_u64(&table, base + 16)?)
        };
        checked_file_range(offset, length, file_len)?;
        return Ok((offset, length));
    }
    Err(SdkError::IdentityMismatch)
}

fn code_signature_range(
    file: &mut File,
    slice_offset: u64,
    slice_len: u64,
) -> SdkResult<(u64, u64)> {
    let header = read_bounded_range(file, slice_offset, 32, 32)?;
    let magic = le_u32(&header, 0)?;
    let header_len = match magic {
        MH_MAGIC => 28_u64,
        MH_MAGIC_64 => 32_u64,
        _ => return Err(SdkError::IdentityMismatch),
    };
    let commands = usize::try_from(le_u32(&header, 16)?).map_err(|_| SdkError::IdentityMismatch)?;
    let commands_len =
        usize::try_from(le_u32(&header, 20)?).map_err(|_| SdkError::IdentityMismatch)?;
    if commands == 0 || commands_len > MAX_LOAD_COMMAND_BYTES {
        return Err(SdkError::IdentityMismatch);
    }
    let commands_bytes = read_bounded_range(
        file,
        slice_offset
            .checked_add(header_len)
            .ok_or(SdkError::IdentityMismatch)?,
        commands_len as u64,
        MAX_LOAD_COMMAND_BYTES,
    )?;
    let mut cursor = 0_usize;
    for _ in 0..commands {
        let command = le_u32(&commands_bytes, cursor)?;
        let command_len = usize::try_from(le_u32(&commands_bytes, cursor + 4)?)
            .map_err(|_| SdkError::IdentityMismatch)?;
        if command_len < 8
            || cursor
                .checked_add(command_len)
                .is_none_or(|end| end > commands_bytes.len())
        {
            return Err(SdkError::IdentityMismatch);
        }
        if command == LC_CODE_SIGNATURE {
            if command_len < 16 {
                return Err(SdkError::IdentityMismatch);
            }
            let relative_offset = u64::from(le_u32(&commands_bytes, cursor + 8)?);
            let length = u64::from(le_u32(&commands_bytes, cursor + 12)?);
            let relative_end = relative_offset
                .checked_add(length)
                .ok_or(SdkError::IdentityMismatch)?;
            if length == 0 || relative_end > slice_len {
                return Err(SdkError::IdentityMismatch);
            }
            return Ok((
                slice_offset
                    .checked_add(relative_offset)
                    .ok_or(SdkError::IdentityMismatch)?,
                length,
            ));
        }
        cursor += command_len;
    }
    Err(SdkError::IdentityMismatch)
}

fn parse_superblob(signature: &[u8]) -> SdkResult<CodeDirectoryHash> {
    if be_u32(signature, 0)? != CSMAGIC_EMBEDDED_SIGNATURE {
        return Err(SdkError::IdentityMismatch);
    }
    let declared_len =
        usize::try_from(be_u32(signature, 4)?).map_err(|_| SdkError::IdentityMismatch)?;
    if signature.get(declared_len..).is_none_or(|padding| {
        padding.len() > MAX_SUPERBLOB_PADDING_BYTES || padding.iter().any(|byte| *byte != 0)
    }) {
        return Err(SdkError::IdentityMismatch);
    }
    let signature = signature
        .get(..declared_len)
        .ok_or(SdkError::IdentityMismatch)?;
    let entries = usize::try_from(be_u32(signature, 8)?).map_err(|_| SdkError::IdentityMismatch)?;
    if entries == 0
        || 12_usize
            .checked_add(entries.checked_mul(8).ok_or(SdkError::IdentityMismatch)?)
            .is_none_or(|end| end > signature.len())
    {
        return Err(SdkError::IdentityMismatch);
    }
    let mut selected = None;
    for index in 0..entries {
        let entry = 12 + index * 8;
        let slot = be_u32(signature, entry)?;
        if slot != CSSLOT_CODEDIRECTORY
            && !(CSSLOT_ALTERNATE_CODEDIRECTORIES..=CSSLOT_ALTERNATE_CODEDIRECTORY_LIMIT)
                .contains(&slot)
        {
            continue;
        }
        let offset = usize::try_from(be_u32(signature, entry + 4)?)
            .map_err(|_| SdkError::IdentityMismatch)?;
        if be_u32(signature, offset)? != CSMAGIC_CODEDIRECTORY {
            return Err(SdkError::IdentityMismatch);
        }
        let length = usize::try_from(be_u32(signature, offset + 4)?)
            .map_err(|_| SdkError::IdentityMismatch)?;
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= signature.len())
            .ok_or(SdkError::IdentityMismatch)?;
        let directory = signature
            .get(offset..end)
            .ok_or(SdkError::IdentityMismatch)?;
        let hash_type = *directory.get(37).ok_or(SdkError::IdentityMismatch)?;
        if !matches!(hash_type, CS_HASHTYPE_SHA256 | CS_HASHTYPE_SHA256_TRUNCATED) {
            continue;
        }
        let digest = Sha256::digest(directory);
        let mut cdhash = [0_u8; 20];
        cdhash.copy_from_slice(&digest[..20]);
        // Prefer the canonical directory when it already uses SHA-256. Otherwise
        // retain the strongest alternate supported by modern macOS.
        if slot == CSSLOT_CODEDIRECTORY {
            return Ok(CodeDirectoryHash(cdhash));
        }
        selected = Some(CodeDirectoryHash(cdhash));
    }
    selected.ok_or(SdkError::IdentityMismatch)
}

fn read_bounded_range(
    file: &mut File,
    offset: u64,
    length: u64,
    maximum: usize,
) -> SdkResult<Vec<u8>> {
    let length = usize::try_from(length).map_err(|_| SdkError::IdentityMismatch)?;
    if length == 0 || length > maximum {
        return Err(SdkError::IdentityMismatch);
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| SdkError::IdentityMismatch)?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)
        .map_err(|_| SdkError::IdentityMismatch)?;
    Ok(bytes)
}

fn checked_file_range(offset: u64, length: u64, file_len: u64) -> SdkResult<()> {
    if length == 0 || offset.checked_add(length).is_none_or(|end| end > file_len) {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

fn be_u32(bytes: &[u8], offset: usize) -> SdkResult<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(SdkError::IdentityMismatch)?
        .try_into()
        .map_err(|_| SdkError::IdentityMismatch)?;
    Ok(u32::from_be_bytes(value))
}

fn be_u64(bytes: &[u8], offset: usize) -> SdkResult<u64> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(SdkError::IdentityMismatch)?
        .try_into()
        .map_err(|_| SdkError::IdentityMismatch)?;
    Ok(u64::from_be_bytes(value))
}

fn le_u32(bytes: &[u8], offset: usize) -> SdkResult<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(SdkError::IdentityMismatch)?
        .try_into()
        .map_err(|_| SdkError::IdentityMismatch)?;
    Ok(u32::from_le_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_test_image_has_a_bounded_code_directory_identity() {
        let mut executable = File::open(std::env::current_exe().expect("test executable"))
            .expect("open test executable");
        let identity = code_directory_hash(&mut executable).expect("code directory identity");
        assert!(identity.requirement().starts_with("cdhash H\""));
        assert_eq!(identity.requirement().len(), 50);
    }

    #[test]
    fn bounded_zero_alignment_padding_is_accepted() {
        let mut executable = File::open(std::env::current_exe().expect("test executable"))
            .expect("open test executable");
        let file_len = executable.metadata().expect("image metadata").len();
        let (slice_offset, slice_len) =
            native_slice(&mut executable, file_len).expect("native slice");
        let (signature_offset, signature_len) =
            code_signature_range(&mut executable, slice_offset, slice_len)
                .expect("signature range");
        let mut signature = read_bounded_range(
            &mut executable,
            signature_offset,
            signature_len,
            MAX_CODE_SIGNATURE_BYTES,
        )
        .expect("signature bytes");
        let declared_len = usize::try_from(be_u32(&signature, 4).expect("declared length"))
            .expect("bounded declared length");
        signature.truncate(declared_len);
        let expected = parse_superblob(&signature).expect("exact superblob");

        signature.extend([0_u8; MAX_SUPERBLOB_PADDING_BYTES]);
        assert_eq!(
            parse_superblob(&signature).expect("zero alignment padding"),
            expected
        );

        signature.push(0);
        assert!(matches!(
            parse_superblob(&signature),
            Err(SdkError::IdentityMismatch)
        ));
        signature.truncate(declared_len + 1);
        *signature.last_mut().expect("padding byte") = 1;
        assert!(matches!(
            parse_superblob(&signature),
            Err(SdkError::IdentityMismatch)
        ));
    }

    #[test]
    fn malformed_images_fail_closed() {
        let mut file = tempfile::tempfile().expect("temporary image");
        std::io::Write::write_all(&mut file, b"not-a-mach-o").expect("write malformed image");
        assert!(matches!(
            code_directory_hash(&mut file),
            Err(SdkError::IdentityMismatch)
        ));
    }
}
