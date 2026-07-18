use super::*;

/// Strict declarative skill manifest. Skills carry context, never executable privilege.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillManifest {
    /// Stable skill identifier.
    pub name: String,
    /// Human-readable version.
    pub version: String,
    /// Bounded discovery summary.
    pub description: String,
    /// Prompt terms that may activate the skill.
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Tool names that must already be active; skills never activate tools.
    #[serde(default)]
    pub required_tools: Vec<String>,
    /// Declarative labels supplied to policy as context only.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Whether the instructions require no network or external integration.
    #[serde(default = "default_true")]
    pub offline_compatible: bool,
}

fn default_true() -> bool {
    true
}

/// Loaded data-only skill and its bounded filesystem provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRecord {
    /// Validated manifest.
    pub manifest: SkillManifest,
    /// Prompt instructions with frontmatter removed.
    pub instructions: String,
    /// Stable provenance label such as `repository:name`.
    pub source: String,
    /// Canonical resource root used only by the trusted resource service.
    pub resource_root: String,
}

/// One deterministic duplicate-resolution result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDuplicate {
    /// Duplicated skill name.
    pub name: String,
    /// Source selected by configured precedence.
    pub selected_source: String,
    /// Every source in precedence order.
    pub sources: Vec<String>,
}

/// Safe metadata for one available or active skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillMetadata {
    /// Skill name.
    pub name: String,
    /// Skill version.
    pub version: String,
    /// Skill description.
    pub description: String,
    /// Provenance label.
    pub source: String,
}

/// Result of deterministic prompt-context skill composition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillComposition {
    /// Original instructions plus bounded skill context.
    pub instructions: String,
    /// Metadata for every enabled skill.
    pub available_skills: Vec<SkillMetadata>,
    /// Metadata for skills activated on this turn.
    pub active_skills: Vec<SkillMetadata>,
}

/// One bounded resource visible under an active data-only skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceEntry {
    /// POSIX path relative to the skill root.
    pub path: String,
    /// File size before reading.
    pub size: u64,
    /// Allowed top-level resource directory.
    pub kind: String,
}

/// One bounded UTF-8 resource read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceRead {
    /// POSIX path relative to the skill root.
    pub path: String,
    /// Exact UTF-8 byte length.
    pub size: u64,
    /// Released text content.
    pub content: String,
}

/// Metadata-only inventory entry for one authorable skill file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFileEntry {
    /// POSIX path relative to the skill root.
    pub path: String,
    /// Exact file size.
    pub size: u64,
    /// SHA-256 of the file bytes.
    pub sha256: String,
}

/// Bounded inspection result for one installed or local skill directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillInspection {
    /// Validated manifest.
    pub manifest: SkillManifest,
    /// Stable source label without instruction content.
    pub source: String,
    /// Deterministic metadata-only file inventory.
    pub files: Vec<SkillFileEntry>,
    /// Hash over the validated manifest, instructions, and file inventory.
    pub content_sha256: String,
}

/// One bounded UTF-8 authoring read from an installed user skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFileRead {
    /// Installed skill name.
    pub name: String,
    /// POSIX path relative to its root.
    pub path: String,
    /// Exact UTF-8 byte length.
    pub size: u64,
    /// SHA-256 used for optimistic writes.
    pub sha256: String,
    /// Released text content.
    pub content: String,
}

/// Result of an atomic optimistic-concurrency authoring write.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillWriteResult {
    /// Installed skill name.
    pub name: String,
    /// POSIX path relative to its root.
    pub path: String,
    /// Hash observed before replacement, absent for a new file.
    pub previous_sha256: Option<String>,
    /// Hash of the committed content.
    pub sha256: String,
    /// Whether the file was newly created.
    pub created: bool,
}

/// Result of creating a new installed data-only skill skeleton.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillScaffoldResult {
    /// Installed skill name.
    pub name: String,
    /// Files created relative to the skill root.
    pub files: Vec<String>,
    /// Hash of the validated installed skill.
    pub content_sha256: String,
}

/// Result of validating an installed or workspace-local skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillValidationResult {
    /// Validated skill name.
    pub name: String,
    /// Stable source label.
    pub source: String,
    /// Deterministic file count.
    pub file_count: usize,
    /// Hash of the validated skill.
    pub content_sha256: String,
}

/// Result of installing a validated workspace-local skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillInstallResult {
    /// Installed skill name.
    pub name: String,
    /// Source hash copied into the user library.
    pub content_sha256: String,
    /// Deterministic number of installed files.
    pub file_count: usize,
}
