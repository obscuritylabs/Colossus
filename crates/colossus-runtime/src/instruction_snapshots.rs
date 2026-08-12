use super::*;
use std::io::Read as _;

const AGENTS_FILE_NAME: &str = "AGENTS.md";
const MAX_AGENTS_FILE_BYTES: usize = 64 * 1024;
const MAX_AGENTS_TOTAL_BYTES: usize = 128 * 1024;
const SNAPSHOT_VERSION: u16 = 1;
const SNAPSHOT_HASH_DOMAIN: &[u8] = b"colossus-instruction-snapshot-v1\0";
const SNAPSHOT_STREAM_PREFIX: &str = "instruction-snapshot:";
const SNAPSHOT_EVENT: &str = "instruction.snapshot.v1";

tokio::task_local! {
    static ACTIVE_INSTRUCTION_SNAPSHOT: Arc<InstructionSnapshot>;
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InstructionSourceKind {
    Home,
    Workspace,
    Invocation,
    RuntimeMode,
}

impl InstructionSourceKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Home => "home AGENTS.md",
            Self::Workspace => "workspace AGENTS.md",
            Self::Invocation => "invocation instructions",
            Self::RuntimeMode => "runtime mode instructions",
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstructionSourceSnapshot {
    kind: InstructionSourceKind,
    sha256: String,
    contents: String,
}

impl InstructionSourceSnapshot {
    fn new(kind: InstructionSourceKind, contents: String) -> Self {
        Self {
            kind,
            sha256: hex::encode(Sha256::digest(contents.as_bytes())),
            contents,
        }
    }
}

/// One immutable, ordered set of model-visible instructions for a top-level run.
///
/// Full contents are intentionally private to runtime composition and the encrypted or
/// explicitly plaintext canonical journal. External surfaces receive only source labels
/// and hashes, never this value.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstructionSnapshot {
    version: u16,
    id: String,
    sources: Vec<InstructionSourceSnapshot>,
}

/// File and invocation layers captured at the start of one top-level run.
///
/// Goal creation assigns its durable identifier after the run has started. Keeping these
/// layers in a pre-finalization value lets Goal Mode add that immutable identifier without
/// re-reading either AGENTS.md file or creating a second snapshot identity.
pub(super) struct CapturedAgentInstructions {
    sources: Vec<InstructionSourceSnapshot>,
}

impl std::fmt::Debug for InstructionSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InstructionSnapshot")
            .field("version", &self.version)
            .field("id", &self.id)
            .field(
                "sources",
                &self
                    .sources
                    .iter()
                    .map(|source| (source.kind.label(), source.sha256.as_str()))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl InstructionSnapshot {
    pub(super) fn capture(
        home: Option<&ConfinedRoot>,
        workspace: &Path,
        invocation: &str,
        runtime_mode: &str,
    ) -> Result<Self, RuntimeError> {
        Ok(Self::capture_layers(home, workspace, invocation)?.into_snapshot(runtime_mode))
    }

    fn capture_layers(
        home: Option<&ConfinedRoot>,
        workspace: &Path,
        invocation: &str,
    ) -> Result<CapturedAgentInstructions, RuntimeError> {
        let mut sources = Vec::with_capacity(3);
        let mut agents_bytes = 0_usize;
        if let Some(home) = home
            && let Some(contents) = read_optional_home_agents_file(home)?
        {
            agents_bytes = agents_bytes.saturating_add(contents.len());
            sources.push(InstructionSourceSnapshot::new(
                InstructionSourceKind::Home,
                contents,
            ));
        }
        if let Some(contents) = read_optional_agents_file(&workspace.join(AGENTS_FILE_NAME))? {
            agents_bytes = agents_bytes.saturating_add(contents.len());
            sources.push(InstructionSourceSnapshot::new(
                InstructionSourceKind::Workspace,
                contents,
            ));
        }
        if agents_bytes > MAX_AGENTS_TOTAL_BYTES {
            return Err(RuntimeError::Config(format!(
                "combined AGENTS.md content exceeds {MAX_AGENTS_TOTAL_BYTES} bytes"
            )));
        }
        sources.push(InstructionSourceSnapshot::new(
            InstructionSourceKind::Invocation,
            invocation.into(),
        ));
        Ok(CapturedAgentInstructions { sources })
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn compose(&self) -> String {
        compose_sources(&self.sources)
    }

    fn compose_without_runtime_mode(&self) -> String {
        let end = if self
            .sources
            .last()
            .is_some_and(|source| source.kind == InstructionSourceKind::RuntimeMode)
        {
            self.sources.len().saturating_sub(1)
        } else {
            self.sources.len()
        };
        compose_sources(&self.sources[..end])
    }

    fn runtime_mode(&self) -> &str {
        self.sources
            .last()
            .filter(|source| source.kind == InstructionSourceKind::RuntimeMode)
            .map_or("", |source| source.contents.as_str())
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        if self.version != SNAPSHOT_VERSION || !valid_snapshot_id(&self.id) {
            return Err(snapshot_verification_error());
        }
        let mut prior = None;
        let mut agents_bytes = 0_usize;
        for source in &self.sources {
            let order = match source.kind {
                InstructionSourceKind::Home => 0_u8,
                InstructionSourceKind::Workspace => 1,
                InstructionSourceKind::Invocation => 2,
                InstructionSourceKind::RuntimeMode => 3,
            };
            if prior.is_some_and(|prior| order <= prior)
                || source.sha256 != hex::encode(Sha256::digest(source.contents.as_bytes()))
            {
                return Err(snapshot_verification_error());
            }
            prior = Some(order);
            if matches!(
                source.kind,
                InstructionSourceKind::Home | InstructionSourceKind::Workspace
            ) {
                if source.contents.len() > MAX_AGENTS_FILE_BYTES {
                    return Err(snapshot_verification_error());
                }
                agents_bytes = agents_bytes.saturating_add(source.contents.len());
            }
        }
        if !self
            .sources
            .last()
            .is_some_and(|source| source.kind == InstructionSourceKind::RuntimeMode)
            || !self
                .sources
                .iter()
                .any(|source| source.kind == InstructionSourceKind::Invocation)
            || agents_bytes > MAX_AGENTS_TOTAL_BYTES
            || snapshot_id(&self.sources) != self.id
        {
            return Err(snapshot_verification_error());
        }
        Ok(())
    }

    fn provenance(&self) -> Vec<(&'static str, &str)> {
        self.sources
            .iter()
            .map(|source| (source.kind.label(), source.sha256.as_str()))
            .collect()
    }

    pub(super) fn automatic_source_diagnostics(&self) -> Value {
        let sources = self
            .provenance()
            .into_iter()
            .filter(|(label, _)| {
                *label != InstructionSourceKind::Invocation.label()
                    && *label != InstructionSourceKind::RuntimeMode.label()
            })
            .map(|(label, sha256)| json!({"source": label, "sha256": sha256}))
            .collect::<Vec<_>>();
        json!({
            "load_order": ["home AGENTS.md", "workspace AGENTS.md", "invocation instructions", "runtime mode instructions"],
            "snapshot_refresh": "top_level_run",
            "sources": sources,
        })
    }
}

pub(super) struct PreparedAgentInstructions {
    pub(super) base_text: String,
    pub(super) text: String,
    runtime_mode: String,
    pub(super) snapshot: Option<Arc<InstructionSnapshot>>,
}

impl CapturedAgentInstructions {
    fn into_snapshot(self, runtime_mode: &str) -> InstructionSnapshot {
        let mut sources = self.sources;
        sources.push(InstructionSourceSnapshot::new(
            InstructionSourceKind::RuntimeMode,
            runtime_mode.into(),
        ));
        InstructionSnapshot {
            version: SNAPSHOT_VERSION,
            id: snapshot_id(&sources),
            sources,
        }
    }

    pub(super) fn finalize(self, runtime_mode: &str) -> PreparedAgentInstructions {
        let snapshot = Arc::new(self.into_snapshot(runtime_mode));
        let base_text = snapshot.compose_without_runtime_mode();
        let text = snapshot.compose();
        PreparedAgentInstructions {
            base_text,
            text,
            runtime_mode: snapshot.runtime_mode().into(),
            snapshot: Some(snapshot),
        }
    }
}

impl PreparedAgentInstructions {
    /// Append the captured immutable runtime layer after caller-composed skill material.
    pub(super) fn complete_composed_base(&self, composed_base: &str) -> String {
        append_runtime_mode(composed_base, &self.runtime_mode)
    }
}

pub(super) struct InstructionSnapshotStore {
    journal: Arc<dyn EventJournal>,
}

impl InstructionSnapshotStore {
    pub(super) fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    pub(super) fn persist(&self, snapshot: &InstructionSnapshot) -> Result<(), RuntimeError> {
        snapshot.validate()?;
        if let Some(existing) = self.load_optional(snapshot.id())? {
            return if existing == *snapshot {
                Ok(())
            } else {
                Err(snapshot_verification_error())
            };
        }
        let stream_id = snapshot_stream(snapshot.id())?;
        let event = NewEvent {
            event_version: 1,
            stream_id: stream_id.clone(),
            expected_stream_version: 0,
            classification: EventClassification::System,
            event_type: SNAPSHOT_EVENT.into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "instruction-snapshot-store".into(),
            },
            context: ExecutionContext {
                correlation_id: stream_id,
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(snapshot)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        };
        match self.journal.append(event) {
            Ok(_) => Ok(()),
            Err(error) => {
                // Concurrent top-level runs can produce the same content-addressed
                // snapshot. Treat the lost optimistic race as success only after the
                // durable value is re-read and proven byte-for-byte equivalent.
                if self
                    .load_optional(snapshot.id())?
                    .is_some_and(|existing| existing == *snapshot)
                {
                    Ok(())
                } else {
                    Err(error.into())
                }
            }
        }
    }

    pub(super) fn load(&self, id: &str) -> Result<InstructionSnapshot, RuntimeError> {
        self.load_optional(id)?
            .ok_or_else(snapshot_verification_error)
    }

    fn load_optional(&self, id: &str) -> Result<Option<InstructionSnapshot>, RuntimeError> {
        let stream_id = snapshot_stream(id)?;
        let events = self.journal.read_stream(&stream_id)?;
        let Some(event) = events.first() else {
            return Ok(None);
        };
        if events.len() != 1
            || event.event_version != 1
            || event.classification != EventClassification::System
            || event.event_type != SNAPSHOT_EVENT
        {
            return Err(snapshot_verification_error());
        }
        let snapshot: InstructionSnapshot =
            serde_json::from_value(self.journal.decrypt_payload(event)?)
                .map_err(|_| snapshot_verification_error())?;
        snapshot.validate()?;
        if snapshot.id != id {
            return Err(snapshot_verification_error());
        }
        Ok(Some(snapshot))
    }
}

impl Runtime {
    pub(super) fn capture_agent_instructions(
        &self,
        invocation: &str,
    ) -> Result<CapturedAgentInstructions, RuntimeError> {
        self._workspace_lease.identity().revalidate()?;
        let captured = capture_layers_for_run(
            self.colossus_home_root.as_ref(),
            &self.workspace,
            invocation,
            self.automatic_agent_instructions,
        )?;
        // The retained lease binds the workspace object. Revalidate after path-based
        // instruction reads so a rename/replacement race can never reach the provider.
        self._workspace_lease.identity().revalidate()?;
        Ok(captured)
    }

    pub(super) fn prepare_agent_instructions(
        &self,
        invocation: &str,
        runtime_mode: &str,
    ) -> Result<PreparedAgentInstructions, RuntimeError> {
        Ok(self
            .capture_agent_instructions(invocation)?
            .finalize(runtime_mode))
    }
}

fn capture_layers_for_run(
    home: Option<&ConfinedRoot>,
    workspace: &Path,
    invocation: &str,
    automatic_agent_instructions: bool,
) -> Result<CapturedAgentInstructions, RuntimeError> {
    if automatic_agent_instructions {
        InstructionSnapshot::capture_layers(home, workspace, invocation)
    } else {
        Ok(CapturedAgentInstructions {
            sources: vec![InstructionSourceSnapshot::new(
                InstructionSourceKind::Invocation,
                invocation.into(),
            )],
        })
    }
}

pub(super) async fn scope_instruction_snapshot<F, T>(
    snapshot: Option<Arc<InstructionSnapshot>>,
    future: F,
) -> T
where
    F: Future<Output = T>,
{
    match snapshot {
        Some(snapshot) => ACTIVE_INSTRUCTION_SNAPSHOT.scope(snapshot, future).await,
        None => future.await,
    }
}

pub(super) fn active_instruction_snapshot() -> Option<Arc<InstructionSnapshot>> {
    ACTIVE_INSTRUCTION_SNAPSHOT.try_with(Arc::clone).ok()
}

fn compose_sources(sources: &[InstructionSourceSnapshot]) -> String {
    let mut composed = String::new();
    for source in sources {
        if source.contents.is_empty() {
            continue;
        }
        if !composed.is_empty() {
            composed.push_str("\n\n");
        }
        composed.push_str("[Colossus ");
        composed.push_str(source.kind.label());
        composed.push_str("]\n");
        composed.push_str(&source.contents);
    }
    composed
}

fn append_runtime_mode(base: &str, runtime_mode: &str) -> String {
    if runtime_mode.is_empty() {
        return base.into();
    }
    let source =
        InstructionSourceSnapshot::new(InstructionSourceKind::RuntimeMode, runtime_mode.into());
    if base.is_empty() {
        compose_sources(&[source])
    } else {
        format!("{base}\n\n{}", compose_sources(&[source]))
    }
}

fn snapshot_id(sources: &[InstructionSourceSnapshot]) -> String {
    let mut digest = Sha256::new();
    digest.update(SNAPSHOT_HASH_DOMAIN);
    for source in sources {
        let kind = match source.kind {
            InstructionSourceKind::Home => 0_u8,
            InstructionSourceKind::Workspace => 1,
            InstructionSourceKind::Invocation => 2,
            InstructionSourceKind::RuntimeMode => 3,
        };
        digest.update([kind]);
        digest.update(
            u64::try_from(source.contents.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        digest.update(source.contents.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn valid_snapshot_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn snapshot_stream(id: &str) -> Result<String, RuntimeError> {
    valid_snapshot_id(id)
        .then(|| format!("{SNAPSHOT_STREAM_PREFIX}{id}"))
        .ok_or_else(snapshot_verification_error)
}

fn snapshot_verification_error() -> RuntimeError {
    RuntimeError::Config("durable instruction snapshot failed verification".into())
}

fn read_optional_agents_file(path: &Path) -> Result<Option<String>, RuntimeError> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(agents_file_error(path, &error.to_string())),
    };
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(agents_file_error(
            path,
            "must be a regular file and must not be a symbolic link",
        ));
    }
    if before.len() > MAX_AGENTS_FILE_BYTES as u64 {
        return Err(agents_file_error(
            path,
            &format!("exceeds the {MAX_AGENTS_FILE_BYTES} byte limit"),
        ));
    }

    #[cfg(unix)]
    let (mut file, opened_identity) = {
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NONBLOCK)
                    .bits() as i32,
            )
            .open(path)
            .map_err(|error| agents_file_error(path, &error.to_string()))?;
        let metadata = file
            .metadata()
            .map_err(|error| agents_file_error(path, &error.to_string()))?;
        if !metadata.is_file() || before.dev() != metadata.dev() || before.ino() != metadata.ino() {
            return Err(agents_file_error(path, "changed while it was being opened"));
        }
        (file, (metadata.dev(), metadata.ino()))
    };

    #[cfg(windows)]
    let (mut file, binding) = {
        let binding = colossus_windows_native::BoundPath::open_file(path)
            .map_err(|error| agents_file_error(path, &error.to_string()))?;
        let file = binding
            .try_clone_file()
            .map_err(|error| agents_file_error(path, &error.to_string()))?;
        (file, binding)
    };

    #[cfg(not(any(unix, windows)))]
    let mut file: fs::File = return Err(agents_file_error(
        path,
        "secure no-follow reads are unsupported on this platform",
    ));

    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take((MAX_AGENTS_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| agents_file_error(path, &error.to_string()))?;
    if bytes.len() > MAX_AGENTS_FILE_BYTES {
        return Err(agents_file_error(
            path,
            &format!("exceeds the {MAX_AGENTS_FILE_BYTES} byte limit"),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        let opened = file
            .metadata()
            .map_err(|error| agents_file_error(path, &error.to_string()))?;
        let after = fs::symlink_metadata(path)
            .map_err(|error| agents_file_error(path, &error.to_string()))?;
        if !opened.is_file()
            || after.file_type().is_symlink()
            || !after.is_file()
            || opened_identity != (opened.dev(), opened.ino())
            || opened_identity != (after.dev(), after.ino())
            || opened.len() != bytes.len() as u64
        {
            return Err(agents_file_error(path, "changed while it was being read"));
        }
    }
    #[cfg(windows)]
    binding
        .revalidate()
        .map_err(|error| agents_file_error(path, &error.to_string()))?;

    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| agents_file_error(path, "must contain valid UTF-8"))
}

fn read_optional_home_agents_file(root: &ConfinedRoot) -> Result<Option<String>, RuntimeError> {
    root.revalidate().map_err(|error| {
        agents_file_error(&root.path().join(AGENTS_FILE_NAME), &error.to_string())
    })?;
    let confined = match root.open_existing_file(Path::new(AGENTS_FILE_NAME)) {
        Ok(file) => file,
        Err(colossus_home::HomeError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            root.revalidate().map_err(|error| {
                agents_file_error(&root.path().join(AGENTS_FILE_NAME), &error.to_string())
            })?;
            return Ok(None);
        }
        Err(error) => {
            return Err(agents_file_error(
                &root.path().join(AGENTS_FILE_NAME),
                &error.to_string(),
            ));
        }
    };
    let path = confined.path().to_owned();
    let mut file = confined.into_file();
    let before = file
        .metadata()
        .map_err(|error| agents_file_error(&path, &error.to_string()))?;
    if before.len() > MAX_AGENTS_FILE_BYTES as u64 {
        return Err(agents_file_error(
            &path,
            &format!("exceeds the {MAX_AGENTS_FILE_BYTES} byte limit"),
        ));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take((MAX_AGENTS_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| agents_file_error(&path, &error.to_string()))?;
    if bytes.len() > MAX_AGENTS_FILE_BYTES {
        return Err(agents_file_error(
            &path,
            &format!("exceeds the {MAX_AGENTS_FILE_BYTES} byte limit"),
        ));
    }
    let after = file
        .metadata()
        .map_err(|error| agents_file_error(&path, &error.to_string()))?;
    if !after.is_file() || before.len() != after.len() || after.len() != bytes.len() as u64 {
        return Err(agents_file_error(&path, "changed while it was being read"));
    }
    root.revalidate()
        .map_err(|error| agents_file_error(&path, &error.to_string()))?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| agents_file_error(&path, "must contain valid UTF-8"))
}

fn agents_file_error(path: &Path, reason: &str) -> RuntimeError {
    RuntimeError::Config(format!(
        "AGENTS.md at {} is unsafe or unreadable: {reason}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_testkit::InMemoryEventJournal;

    #[test]
    fn capture_orders_all_instruction_layers_with_hash_only_provenance() {
        let home = tempfile::tempdir().expect("home");
        let workspace = tempfile::tempdir().expect("workspace");
        let home_path = home.path().canonicalize().expect("canonical home");
        #[cfg(unix)]
        fs::set_permissions(
            &home_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("private home permissions");
        fs::write(home_path.join(AGENTS_FILE_NAME), "home rules").expect("home instructions");
        fs::write(workspace.path().join(AGENTS_FILE_NAME), "workspace rules")
            .expect("workspace instructions");
        let home_root = ConfinedRoot::bind(&home_path).expect("bound home");

        let snapshot = InstructionSnapshot::capture(
            Some(&home_root),
            workspace.path(),
            "invocation rules",
            "immutable Plan Mode rules",
        )
        .expect("snapshot");
        let composed = snapshot.compose();
        let home_offset = composed.find("home rules").expect("home");
        let workspace_offset = composed.find("workspace rules").expect("workspace");
        let invocation_offset = composed.find("invocation rules").expect("invocation");
        let runtime_offset = composed
            .find("immutable Plan Mode rules")
            .expect("runtime mode");
        assert!(
            home_offset < workspace_offset
                && workspace_offset < invocation_offset
                && invocation_offset < runtime_offset
        );
        assert_eq!(
            snapshot
                .provenance()
                .into_iter()
                .map(|(label, hash)| (label, hash.len()))
                .collect::<Vec<_>>(),
            vec![
                ("home AGENTS.md", 64),
                ("workspace AGENTS.md", 64),
                ("invocation instructions", 64),
                ("runtime mode instructions", 64),
            ]
        );
        let diagnostics = snapshot.automatic_source_diagnostics().to_string();
        assert!(diagnostics.contains("home AGENTS.md"));
        assert!(diagnostics.contains("workspace AGENTS.md"));
        assert!(!diagnostics.contains("home rules"));
        assert!(!diagnostics.contains("workspace rules"));
        assert!(!diagnostics.contains("invocation rules"));
        assert!(!diagnostics.contains("immutable Plan Mode rules"));
        let different_mode = InstructionSnapshot::capture(
            Some(&home_root),
            workspace.path(),
            "invocation rules",
            "different immutable mode",
        )
        .expect("different mode snapshot");
        assert_ne!(
            snapshot.id(),
            different_mode.id(),
            "runtime-mode content must contribute to the durable snapshot identity"
        );
    }

    #[test]
    fn capture_reloads_files_for_each_top_level_snapshot() {
        let workspace = tempfile::tempdir().expect("workspace");
        let path = workspace.path().join(AGENTS_FILE_NAME);
        fs::write(&path, "first rules").expect("initial instructions");
        let first = InstructionSnapshot::capture(None, workspace.path(), "run", "")
            .expect("first snapshot");
        fs::write(&path, "second rules").expect("updated instructions");
        let second = InstructionSnapshot::capture(None, workspace.path(), "run", "")
            .expect("second snapshot");

        assert!(first.compose().contains("first rules"));
        assert!(!first.compose().contains("second rules"));
        assert!(second.compose().contains("second rules"));
        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn capture_rejects_oversized_and_non_utf8_files() {
        let workspace = tempfile::tempdir().expect("workspace");
        let path = workspace.path().join(AGENTS_FILE_NAME);
        fs::write(&path, vec![b'a'; MAX_AGENTS_FILE_BYTES + 1]).expect("oversized file");
        assert!(
            InstructionSnapshot::capture(None, workspace.path(), "run", "")
                .expect_err("oversized instructions must fail")
                .to_string()
                .contains("byte limit")
        );
        fs::write(&path, [0xff, 0xfe]).expect("non-UTF-8 file");
        assert!(
            InstructionSnapshot::capture(None, workspace.path(), "run", "")
                .expect_err("non-UTF-8 instructions must fail")
                .to_string()
                .contains("valid UTF-8")
        );
    }

    #[test]
    fn capture_rejects_present_non_regular_agents_path() {
        let workspace = tempfile::tempdir().expect("workspace");
        fs::create_dir(workspace.path().join(AGENTS_FILE_NAME)).expect("directory");

        assert!(
            InstructionSnapshot::capture(None, workspace.path(), "run", "")
                .expect_err("a directory must not be read as instructions")
                .to_string()
                .contains("must be a regular file")
        );
    }

    #[cfg(unix)]
    #[test]
    fn capture_rejects_present_symlinked_agents_file() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().expect("workspace");
        let target = workspace.path().join("instructions.txt");
        fs::write(&target, "linked rules").expect("target");
        symlink(&target, workspace.path().join(AGENTS_FILE_NAME)).expect("symlink");

        assert!(
            InstructionSnapshot::capture(None, workspace.path(), "run", "")
                .expect_err("symlinked instructions must fail")
                .to_string()
                .contains("must not be a symbolic link")
        );
    }

    #[cfg(unix)]
    #[test]
    fn home_capture_rejects_a_replaced_bound_namespace() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().canonicalize().expect("canonical root");
        let home = root.join("home");
        let displaced = root.join("displaced-home");
        fs::create_dir(&home).expect("home");
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).expect("home permissions");
        fs::write(home.join(AGENTS_FILE_NAME), "original rules").expect("original rules");
        let home_root = ConfinedRoot::bind(&home).expect("bound home");

        fs::rename(&home, &displaced).expect("displace home");
        fs::create_dir(&home).expect("replacement home");
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700))
            .expect("replacement permissions");
        fs::write(home.join(AGENTS_FILE_NAME), "replacement rules").expect("replacement rules");
        let workspace = tempfile::tempdir().expect("workspace");

        assert!(
            InstructionSnapshot::capture(Some(&home_root), workspace.path(), "run", "")
                .expect_err("a replaced home namespace must fail closed")
                .to_string()
                .contains("unsafe")
        );
    }

    #[test]
    fn durable_store_round_trips_exact_material_without_debug_disclosure() {
        let workspace = tempfile::tempdir().expect("workspace");
        fs::write(
            workspace.path().join(AGENTS_FILE_NAME),
            "private repository rules",
        )
        .expect("instructions");
        let snapshot = InstructionSnapshot::capture(
            None,
            workspace.path(),
            "caller rules",
            "immutable Goal Mode rules",
        )
        .expect("snapshot");
        assert!(!format!("{snapshot:?}").contains("private repository rules"));
        assert!(!format!("{snapshot:?}").contains("caller rules"));
        assert!(!format!("{snapshot:?}").contains("immutable Goal Mode rules"));

        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let store = InstructionSnapshotStore::new(Arc::clone(&journal));
        store.persist(&snapshot).expect("persist snapshot");
        store.persist(&snapshot).expect("idempotent persist");
        assert_eq!(store.load(snapshot.id()).expect("load snapshot"), snapshot);
        assert_eq!(
            journal
                .read_stream(&snapshot_stream(snapshot.id()).expect("stream id"))
                .expect("events")
                .len(),
            1
        );
    }

    #[test]
    fn durable_runtime_mode_does_not_count_against_exact_agents_boundary() {
        let home = tempfile::tempdir().expect("home");
        let workspace = tempfile::tempdir().expect("workspace");
        let home_path = home.path().canonicalize().expect("canonical home");
        #[cfg(unix)]
        fs::set_permissions(
            &home_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("private home permissions");
        fs::write(
            home_path.join(AGENTS_FILE_NAME),
            vec![b'h'; MAX_AGENTS_FILE_BYTES],
        )
        .expect("home instructions");
        fs::write(
            workspace.path().join(AGENTS_FILE_NAME),
            vec![b'w'; MAX_AGENTS_FILE_BYTES],
        )
        .expect("workspace instructions");
        let home_root = ConfinedRoot::bind(&home_path).expect("bound home");
        let snapshot = InstructionSnapshot::capture(
            Some(&home_root),
            workspace.path(),
            "caller rules",
            "nonempty immutable runtime mode",
        )
        .expect("exact-boundary snapshot");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let store = InstructionSnapshotStore::new(journal);

        store.persist(&snapshot).expect("persist exact boundary");
        assert_eq!(
            store.load(snapshot.id()).expect("reload exact boundary"),
            snapshot
        );
    }

    #[tokio::test]
    async fn task_scope_inherits_one_snapshot_without_leaking_to_later_runs() {
        let workspace = tempfile::tempdir().expect("workspace");
        let snapshot = Arc::new(
            InstructionSnapshot::capture(None, workspace.path(), "fixed", "fixed runtime mode")
                .expect("snapshot"),
        );
        let expected = snapshot.id().to_owned();
        let observed = scope_instruction_snapshot(Some(snapshot), async {
            tokio::task::yield_now().await;
            active_instruction_snapshot()
                .expect("active snapshot")
                .id()
                .to_owned()
        })
        .await;
        assert_eq!(observed, expected);
        assert!(active_instruction_snapshot().is_none());
    }

    #[test]
    fn delayed_goal_finalization_uses_the_run_start_file_snapshot() {
        let workspace = tempfile::tempdir().expect("workspace");
        let path = workspace.path().join(AGENTS_FILE_NAME);
        fs::write(&path, "rules at run start").expect("initial instructions");
        let captured = InstructionSnapshot::capture_layers(None, workspace.path(), "goal run")
            .expect("captured layers");

        fs::write(&path, "rules changed during goal creation").expect("changed instructions");
        let prepared = captured.finalize("immutable Goal Mode goal-123");

        assert!(prepared.text.contains("rules at run start"));
        assert!(!prepared.text.contains("rules changed during goal creation"));
        assert!(prepared.text.ends_with("immutable Goal Mode goal-123"));
        assert!(
            prepared
                .snapshot
                .expect("user-facing snapshot")
                .compose()
                .ends_with("immutable Goal Mode goal-123")
        );
    }

    #[test]
    fn trusted_diagnostic_capture_suppresses_only_automatic_agents_layers() {
        let home = tempfile::tempdir().expect("home");
        let workspace = tempfile::tempdir().expect("workspace");
        let home_path = home.path().canonicalize().expect("canonical home");
        #[cfg(unix)]
        fs::set_permissions(
            &home_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("private home permissions");
        fs::write(
            home_path.join(AGENTS_FILE_NAME),
            "hostile home diagnostic sentinel",
        )
        .expect("home instructions");
        fs::write(
            workspace.path().join(AGENTS_FILE_NAME),
            "hostile workspace diagnostic sentinel",
        )
        .expect("workspace instructions");
        let home_root = ConfinedRoot::bind(&home_path).expect("bound home");

        let prepared = capture_layers_for_run(
            Some(&home_root),
            workspace.path(),
            "explicit offline probe",
            false,
        )
        .expect("suppressed capture")
        .finalize("immutable Plan probe mode");

        assert!(!prepared.text.contains("hostile home diagnostic sentinel"));
        assert!(
            !prepared
                .text
                .contains("hostile workspace diagnostic sentinel")
        );
        assert!(prepared.text.contains("explicit offline probe"));
        assert!(prepared.text.ends_with("immutable Plan probe mode"));
    }
}
