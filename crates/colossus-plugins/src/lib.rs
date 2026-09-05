//! Agent Plugins v1 discovery, machine-scoped lifecycle, and OCI distribution.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    AGENT_PLUGIN_MCP_SCHEMA_V1, AGENT_PLUGIN_SCHEMA_V1, Actor, AgentPluginManifest,
    AgentPluginRecord, AgentSkillManifest, PluginComponentDiagnostic, PluginComponentKind,
    PluginComposition, PluginInstallation, PluginMcpServer, PluginMcpTransport,
    PluginResourceEntry, PluginResourceRead, PluginSkillMetadata, PluginSkillRecord, PluginStatus,
    PluginTrustEvidence, PluginValidation,
};
use colossus_journal_redb::{DisabledCheckpointSigner, PlaintextKeyProvider, RedbEventJournal};
use colossus_ports::{EventJournal, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use url::Url;

/// Maximum size of a portable plugin or OCI config manifest.
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
/// Maximum `SKILL.md` size admitted for model disclosure.
pub const MAX_SKILL_BYTES: u64 = 256 * 1024;
/// Maximum size of one regular plugin file.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum extracted size of one whole plugin artifact.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Maximum number of files in one plugin artifact.
pub const MAX_FILES: usize = 10_000;
/// Maximum text-resource preview size.
pub const MAX_RESOURCE_PREVIEW_BYTES: u64 = 64_000;
/// Maximum resource-listing depth below one Agent Skill.
pub const MAX_RESOURCE_DEPTH: usize = 16;
/// Maximum composed system-instruction size after selected skills are loaded.
pub const MAX_COMPOSED_BYTES: usize = 512 * 1024;

const PLUGIN_SCHEMA: &str = include_str!("schemas/plugin.schema.json");
const MCP_SCHEMA: &str = include_str!("schemas/mcp.schema.json");

mod oci;
pub use oci::*;

mod registry;
pub use registry::*;

mod store;
pub use store::*;

mod trust;
pub use trust::*;

mod common;
use common::*;
mod contained;
use contained::{ReadRoot, read_contained};
mod filesystem;
use filesystem::*;
mod discovery;
mod schema;
use discovery::*;
pub use discovery::{load_plugin, validate_plugin};
mod skills;
use skills::*;
mod mcp;
use mcp::*;
mod composition;
pub use composition::compose_plugins;
mod resources;
pub use resources::{list_resources, read_resource};

#[cfg(test)]
mod contained_tests;
#[cfg(test)]
mod tests;
