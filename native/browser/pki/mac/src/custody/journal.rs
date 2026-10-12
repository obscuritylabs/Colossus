//! Fixed protected append-only admitted generation receipts. No key material.
mod model;
use super::{package::Package, platform::Boundary};
use crate::{MutationRequest, RetirementStep, Status};
use colossus_darwin_process::{DarwinAuditPeer, DarwinProcessIdentity};
use colossus_home::ConfinedRoot;
use model::{Binding, Event, Header, Operation, Output, State};
use sha2::{Digest as _, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    os::fd::{AsRawFd as _, BorrowedFd},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::PathBuf,
    time::UNIX_EPOCH,
};
const NAME: &str = "colossus-custody-generation.jsonl";
const LIMIT: u64 = 32 * 1024;
pub(super) struct Journal {
    root: ConfinedRoot,
    path: PathBuf,
    file: File,
    file_binding: Binding,
    state: State,
    header: Header,
    bytes: u64,
    digest: [u8; 32],
    failed: bool,
}
impl Journal {
    pub fn create(
        package: &Package,
        boundary: &Boundary,
        parent: &DarwinAuditPeer,
        generation: &[u8; 32],
        parent_cdhash: &[u8; 20],
    ) -> Result<Self, Status> {
        package.revalidate()?;
        parent.verify().map_err(|_| Status::Denied)?;
        let root = ConfinedRoot::bind(package.custody_root()).map_err(|_| Status::Denied)?;
        let path = root.path().join(NAME);
        match std::fs::symlink_metadata(root.path().join("colossus-tls-identity")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(Status::Denied),
        }
        for entry in std::fs::read_dir(root.path()).map_err(|_| Status::Denied)? {
            if entry
                .map_err(|_| Status::Denied)?
                .file_name()
                .to_string_lossy()
                .starts_with(".retired-tls-identity-")
            {
                return Err(Status::Denied);
            }
        }
        let generation_hash: [u8; 32] = Sha256::digest(generation).into();
        let broker_identity =
            DarwinProcessIdentity::bind(std::process::id()).map_err(|_| Status::Denied)?;
        let broker = DarwinAuditPeer::enroll(&broker_identity, package.broker_code_hash())
            .map_err(|_| Status::Denied)?;
        broker.verify().map_err(|_| Status::Denied)?;
        parent.verify().map_err(|_| Status::Denied)?;
        package.revalidate()?;
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(0x0100 | 0x01000000)
            .open(&path)
            .map_err(|_| Status::Denied)?;
        let file_binding = binding(file.as_raw_fd())?;
        let header = Header {
            schema_version: 1,
            purpose: "colossus-macos-owned-custody-generation".into(),
            package_binding_sha256: hex::encode(package.custody_record_binding()),
            broker_cdhash: hex::encode(package.broker_code_hash()),
            parent_cdhash: hex::encode(parent_cdhash),
            parent_generation_binding_sha256: hex::encode(
                parent
                    .identity()
                    .custody_generation_binding(&generation_hash),
            ),
            generation_sha256: hex::encode(generation_hash),
            broker_pid: broker.identity().pid(),
            broker_generation_binding_sha256: hex::encode(
                broker
                    .identity()
                    .custody_generation_binding(&generation_hash),
            ),
            parent_directory: binding(boundary.parent_fd().as_raw_fd())?,
        };
        let mut journal = Self {
            root,
            path,
            file,
            file_binding,
            state: State::new(),
            header: header.clone(),
            bytes: 0,
            digest: Sha256::digest([]).into(),
            failed: false,
        };
        journal.append(&header)?;
        package.revalidate()?;
        parent.verify().map_err(|_| Status::Denied)?;
        Ok(journal)
    }
    fn revalidate(&self) -> Result<(), Status> {
        self.root.revalidate().map_err(|_| Status::OutcomeUnknown)?;
        let current = binding(self.file.as_raw_fd())?;
        let entry = std::fs::symlink_metadata(&self.path).map_err(|_| Status::OutcomeUnknown)?;
        if current != self.file_binding
            || entry.file_type().is_symlink()
            || entry.dev() != current.device
            || entry.ino() != current.inode
            || entry.nlink() != 1
            || entry.uid() != current.uid
            || entry.mode() & 0o7777 != 0o600
        {
            return Err(Status::OutcomeUnknown);
        }
        Ok(())
    }
    fn append(&mut self, value: &impl serde::Serialize) -> Result<(), Status> {
        if self.failed {
            return Err(Status::OutcomeUnknown);
        }
        let result = (|| {
            self.revalidate()?;
            let mut copy = self.file.try_clone().map_err(|_| Status::OutcomeUnknown)?;
            copy.seek(SeekFrom::Start(0))
                .map_err(|_| Status::OutcomeUnknown)?;
            let mut old = Vec::new();
            copy.take(LIMIT + 1)
                .read_to_end(&mut old)
                .map_err(|_| Status::OutcomeUnknown)?;
            if old.len() as u64 != self.bytes || Sha256::digest(&old).as_slice() != self.digest {
                return Err(Status::OutcomeUnknown);
            }
            if self.bytes > 0 {
                State::replay(&self.header, &old)?;
            }
            let mut bytes = serde_json::to_vec(value).map_err(|_| Status::OutcomeUnknown)?;
            bytes.push(b'\n');
            if bytes.len() > 4096
                || self
                    .file
                    .metadata()
                    .map_err(|_| Status::OutcomeUnknown)?
                    .len()
                    + bytes.len() as u64
                    > LIMIT
            {
                return Err(Status::OutcomeUnknown);
            }
            self.file
                .write_all(&bytes)
                .map_err(|_| Status::OutcomeUnknown)?;
            self.file.sync_all().map_err(|_| Status::OutcomeUnknown)?;
            self.root
                .sync_directory()
                .map_err(|_| Status::OutcomeUnknown)?;
            self.revalidate()?;
            old.extend_from_slice(&bytes);
            self.bytes = old.len() as u64;
            self.digest = Sha256::digest(&old).into();
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub fn pending(&mut self, request: &MutationRequest) -> Result<(), Status> {
        let operation = Operation::from(request.operation);
        let event = Event::Pending {
            sequence: request.sequence,
            operation,
        };
        let next = self.state.transition(&event)?;
        self.append(&event)?;
        self.state = next;
        Ok(())
    }
    pub fn sealed(
        &mut self,
        boundary: &Boundary,
        sequence: u64,
        fds: &[i32],
    ) -> Result<(), Status> {
        let outputs = fds
            .iter()
            .map(|fd| {
                let kind = boundary.descriptor_kind(*fd);
                if !(1..=4).contains(&kind) {
                    return Err(Status::OutcomeUnknown);
                }
                Ok(Output {
                    kind,
                    binding: binding(*fd)?,
                })
            })
            .collect::<Result<Vec<_>, Status>>()?;
        let event = Event::Sealed { sequence, outputs };
        let next = self.state.transition(&event)?;
        self.append(&event)?;
        self.state = next;
        Ok(())
    }
    pub fn retire_unchanged(
        &mut self,
        boundary: &Boundary,
        sequence: u64,
        fds: &[i32],
    ) -> Result<(), Status> {
        let outputs = fds
            .iter()
            .map(|fd| {
                Ok(Output {
                    kind: boundary.descriptor_kind(*fd),
                    binding: binding(*fd)?,
                })
            })
            .collect::<Result<Vec<_>, Status>>()?;
        let event = Event::UnchangedForRetirement { sequence, outputs };
        let next = self.state.transition(&event)?;
        self.append(&event)?;
        self.state = next;
        Ok(())
    }
    pub fn checkpoint(
        &mut self,
        boundary: &Boundary,
        sequence: u64,
        step: RetirementStep,
    ) -> Result<(), Status> {
        let suffix = boundary.retired_suffix().ok_or(Status::OutcomeUnknown)?;
        let suffix = std::str::from_utf8(&suffix)
            .map_err(|_| Status::OutcomeUnknown)?
            .to_owned();
        let step = match step {
            RetirementStep::NamespaceRenamed => 0,
            RetirementStep::StoreRemoved => 1,
            RetirementStep::AuxiliaryRemoved => 2,
            RetirementStep::DirectoryRemoved => 3,
        };
        let event = Event::Retirement {
            sequence,
            step,
            suffix,
        };
        let next = self.state.transition(&event)?;
        self.append(&event)?;
        self.state = next;
        Ok(())
    }
}
fn binding(fd: i32) -> Result<Binding, Status> {
    // SAFETY: these descriptors come only from the retained concrete boundary or
    // synchronous local native receipt callback; cloning never closes the borrow.
    let file = File::from(
        unsafe { BorrowedFd::borrow_raw(fd) }
            .try_clone_to_owned()
            .map_err(|_| Status::OutcomeUnknown)?,
    );
    let metadata = file.metadata().map_err(|_| Status::OutcomeUnknown)?;
    let birth = metadata
        .created()
        .map_err(|_| Status::OutcomeUnknown)?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Status::OutcomeUnknown)?;
    Ok(Binding {
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
        mode: metadata.mode() & 0o7777,
        birth_seconds: birth.as_secs(),
        birth_nanos: birth.subsec_nanos(),
    })
}
