//! Declarative non-executable skills, deterministic resolution, and safe text resources.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    SkillComposition, SkillDuplicate, SkillFileEntry, SkillFileRead, SkillInspection,
    SkillInstallResult, SkillManifest, SkillMetadata, SkillRecord, SkillResourceEntry,
    SkillResourceRead, SkillScaffoldResult, SkillValidationResult, SkillWriteResult, ToolSpec,
};
use colossus_policy::ExecutionPermit;
use colossus_ports::{SkillRepository, StoreError};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_INSTRUCTION_BYTES: u64 = 256 * 1024;
const MAX_COMPOSED_BYTES: usize = 512 * 1024;
const MAX_RESOURCE_BYTES: u64 = 64_000;
const MAX_RESOURCE_ENTRIES: usize = 1_000;
const MAX_RESOURCE_DEPTH: usize = 16;
const MAX_AUTHOR_FILES: usize = 1_000;
const MAX_AUTHOR_FILE_BYTES: u64 = 256 * 1024;
const MAX_AUTHOR_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const RESOURCE_DIRS: [&str; 5] = ["assets", "examples", "references", "scripts", "tests"];

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

#[cfg(test)]
use repository::split_frontmatter;
use repository::{ensure_contained, load_skill, triggers_from_name, valid_skill_name};
use resources::posix_path;

mod repository;
pub use repository::*;

mod composer;
pub use composer::*;

mod resources;
pub use resources::*;

mod authoring;
pub use authoring::*;

#[cfg(test)]
mod tests;
