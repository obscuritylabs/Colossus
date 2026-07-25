use std::{
    cmp::Ordering,
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, SettingsStore, WorkspaceSetting,
        application_support_root, revalidate_workspace,
    },
    dto::CommandErrorDto,
    state::{AppState, MANAGED_TARGET_ID},
};

const MAX_DIRECTORY_ENTRIES: usize = 500;
const MAX_DIRECTORY_SCAN_ENTRIES: usize = 2_048;
const MAX_FILE_BYTES: u64 = 256 * 1_024;
const MAX_RELATIVE_PATH_BYTES: usize = 2_048;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListWorkspaceDirectoryInput {
    pub(crate) workspace_id: String,
    #[serde(default)]
    pub(crate) path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReadWorkspaceFileInput {
    pub(crate) workspace_id: String,
    pub(crate) path: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkspaceEntryKindDto {
    Directory,
    File,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceEntryDto {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) kind: WorkspaceEntryKindDto,
    pub(crate) size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceDirectoryDto {
    pub(crate) path: String,
    pub(crate) entries: Vec<WorkspaceEntryDto>,
    pub(crate) truncated: bool,
    pub(crate) excluded_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFileDto {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) content: String,
    pub(crate) language: String,
    pub(crate) size_bytes: u64,
    pub(crate) line_count: usize,
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_workspace_directory(
    state: State<'_, AppState>,
    request: ListWorkspaceDirectoryInput,
) -> Result<WorkspaceDirectoryDto, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    let workspace = authorize_workspace(&settings, &request.workspace_id)?;
    let root = revalidate_workspace(workspace)?;
    let result = list_directory(&root, &request.path)?;
    ensure_workspace_unchanged(workspace, &root)?;
    let selected = state.selected_target_id().await;
    if selected.as_deref() != Some(MANAGED_TARGET_ID) {
        return Err(files_unavailable());
    }
    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn read_workspace_file(
    state: State<'_, AppState>,
    request: ReadWorkspaceFileInput,
) -> Result<WorkspaceFileDto, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    let workspace = authorize_workspace(&settings, &request.workspace_id)?;
    let root = revalidate_workspace(workspace)?;
    let result = read_file(&root, &request.path)?;
    ensure_workspace_unchanged(workspace, &root)?;
    let selected = state.selected_target_id().await;
    if selected.as_deref() != Some(MANAGED_TARGET_ID) {
        return Err(files_unavailable());
    }
    Ok(result)
}

fn settings_store() -> Result<SettingsStore, CommandErrorDto> {
    SettingsStore::open(application_support_root()?)
}

fn authorize_workspace<'a>(
    settings: &'a DesktopSettings,
    workspace_id: &str,
) -> Result<&'a WorkspaceSetting, CommandErrorDto> {
    if settings.selected_target_id.as_deref() != Some(MANAGED_TARGET_ID)
        || settings.access_profile != AccessProfileSetting::Development
    {
        return Err(files_unavailable());
    }
    let workspace = settings.workspace.as_ref().ok_or_else(files_unavailable)?;
    if workspace.id != workspace_id {
        return Err(CommandErrorDto::invalid(
            "workspaceId",
            "Select the current Managed Local workspace.",
        ));
    }
    Ok(workspace)
}

fn list_directory(root: &Path, relative: &str) -> Result<WorkspaceDirectoryDto, CommandErrorDto> {
    let directory = resolve_relative(root, relative, true)?;
    let metadata = fs::symlink_metadata(&directory).map_err(|_| workspace_read_error())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_path());
    }

    let mut entries = Vec::new();
    let mut excluded_count = 0usize;
    let mut truncated = false;
    let reader = fs::read_dir(&directory).map_err(|_| workspace_read_error())?;
    for (scanned, entry) in reader.enumerate() {
        if scanned == MAX_DIRECTORY_SCAN_ENTRIES {
            truncated = true;
            break;
        }
        let entry = entry.map_err(|_| workspace_read_error())?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            excluded_count += 1;
            continue;
        };
        if hidden_entry(&name) || !renderer_safe_name(&name) {
            excluded_count += 1;
            continue;
        }
        let metadata = entry
            .path()
            .symlink_metadata()
            .map_err(|_| workspace_read_error())?;
        if metadata.file_type().is_symlink() {
            excluded_count += 1;
            continue;
        }
        let kind = if metadata.is_dir() {
            WorkspaceEntryKindDto::Directory
        } else if metadata.is_file() {
            WorkspaceEntryKindDto::File
        } else {
            excluded_count += 1;
            continue;
        };
        entries.push(WorkspaceEntryDto {
            path: join_relative(relative, &name),
            name,
            kind,
            size_bytes: metadata.is_file().then_some(metadata.len()),
        });
    }
    entries.sort_by(compare_entries);
    if entries.len() > MAX_DIRECTORY_ENTRIES {
        entries.truncate(MAX_DIRECTORY_ENTRIES);
        truncated = true;
    }
    Ok(WorkspaceDirectoryDto {
        path: relative.into(),
        entries,
        truncated,
        excluded_count,
    })
}

fn read_file(root: &Path, relative: &str) -> Result<WorkspaceFileDto, CommandErrorDto> {
    let candidate = resolve_relative(root, relative, false)?;
    let before = fs::symlink_metadata(&candidate).map_err(|_| workspace_read_error())?;
    if before.file_type().is_symlink() || !before.is_file() || before.len() > MAX_FILE_BYTES {
        return Err(preview_unavailable());
    }
    let file = open_file_without_following(&candidate)?;
    let opened = file.metadata().map_err(|_| workspace_read_error())?;
    let after = fs::symlink_metadata(&candidate).map_err(|_| workspace_read_error())?;
    if !opened.is_file()
        || after.file_type().is_symlink()
        || !after.is_file()
        || !same_file(&opened, &after)
    {
        return Err(workspace_read_error());
    }

    let mut bytes = Vec::with_capacity(usize::try_from(opened.len()).unwrap_or_default());
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| workspace_read_error())?;
    if bytes.len() as u64 > MAX_FILE_BYTES || bytes.contains(&0) {
        return Err(preview_unavailable());
    }
    let content = String::from_utf8(bytes).map_err(|_| preview_unavailable())?;
    if content.chars().any(unsafe_text_character) {
        return Err(preview_unavailable());
    }
    let name = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(preview_unavailable)?
        .to_owned();
    Ok(WorkspaceFileDto {
        language: language_for(&name).into(),
        line_count: content.split('\n').count(),
        size_bytes: opened.len(),
        name,
        path: relative.into(),
        content,
    })
}

fn resolve_relative(
    root: &Path,
    relative: &str,
    allow_empty: bool,
) -> Result<PathBuf, CommandErrorDto> {
    if relative.len() > MAX_RELATIVE_PATH_BYTES
        || (!allow_empty && relative.is_empty())
        || relative.starts_with('/')
        || relative.ends_with('/')
        || relative.contains('\\')
        || relative.chars().any(char::is_control)
    {
        return Err(invalid_path());
    }
    if relative.is_empty() {
        return Ok(root.to_owned());
    }

    let mut candidate = root.to_owned();
    for component in relative.split('/') {
        if component.is_empty()
            || matches!(component, "." | "..")
            || hidden_entry(component)
            || !renderer_safe_name(component)
        {
            return Err(invalid_path());
        }
        candidate.push(component);
        let metadata = fs::symlink_metadata(&candidate).map_err(|_| workspace_read_error())?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_path());
        }
    }
    let canonical = fs::canonicalize(&candidate).map_err(|_| workspace_read_error())?;
    if canonical != candidate || !canonical.starts_with(root) {
        return Err(invalid_path());
    }
    Ok(canonical)
}

#[cfg(unix)]
fn open_file_without_following(path: &Path) -> Result<File, CommandErrorDto> {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| workspace_read_error())
}

#[cfg(windows)]
fn open_file_without_following(path: &Path) -> Result<File, CommandErrorDto> {
    colossus_windows_native::BoundPath::open_file(path)
        .and_then(|binding| binding.try_clone_file())
        .map_err(|_| workspace_read_error())
}

#[cfg(not(any(unix, windows)))]
fn open_file_without_following(path: &Path) -> Result<File, CommandErrorDto> {
    File::open(path).map_err(|_| workspace_read_error())
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    left.volume_serial_number() == right.volume_serial_number()
        && left.file_index() == right.file_index()
        && left.file_index().is_some()
}

#[cfg(not(any(unix, windows)))]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.created().ok() == right.created().ok()
}

fn ensure_workspace_unchanged(
    workspace: &WorkspaceSetting,
    expected_root: &Path,
) -> Result<(), CommandErrorDto> {
    let current = revalidate_workspace(workspace)?;
    if current == expected_root {
        Ok(())
    } else {
        Err(files_unavailable())
    }
}

fn compare_entries(left: &WorkspaceEntryDto, right: &WorkspaceEntryDto) -> Ordering {
    match (left.kind, right.kind) {
        (WorkspaceEntryKindDto::Directory, WorkspaceEntryKindDto::File) => Ordering::Less,
        (WorkspaceEntryKindDto::File, WorkspaceEntryKindDto::Directory) => Ordering::Greater,
        _ => left
            .name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name)),
    }
}

fn join_relative(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.into()
    } else {
        format!("{parent}/{name}")
    }
}

fn renderer_safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '\u{061c}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                )
        })
}

fn hidden_entry(name: &str) -> bool {
    let lowercase = name.to_ascii_lowercase();
    matches!(
        lowercase.as_str(),
        ".colossus"
            | ".git"
            | ".hg"
            | ".svn"
            | ".netrc"
            | ".npmrc"
            | ".pypirc"
            | ".authinfo"
            | ".authinfo.gpg"
            | ".aws"
            | ".azure"
            | ".docker"
            | ".git-credentials"
            | ".gitconfig"
            | ".gnupg"
            | ".kube"
            | ".password-store"
            | ".ssh"
            | "gcloud"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".venv"
            | "venv"
            | "id_rsa"
            | "id_ed25519"
            | "credentials"
            | "credentials.json"
            | "application_default_credentials.json"
    ) || lowercase == ".env"
        || lowercase.starts_with(".env.")
        || lowercase.starts_with("secrets.")
        || Path::new(&lowercase)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension,
                    "pem" | "key" | "p12" | "pfx" | "jks" | "keystore"
                )
            })
}

fn unsafe_text_character(character: char) -> bool {
    (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn language_for(name: &str) -> &'static str {
    let lowercase = name.to_ascii_lowercase();
    if matches!(lowercase.as_str(), "dockerfile" | "containerfile") {
        return "dockerfile";
    }
    match Path::new(&lowercase)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "mdx" => "markdown",
        "sh" | "bash" | "zsh" => "bash",
        "py" => "python",
        "go" => "go",
        "sql" => "sql",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "proto" => "protobuf",
        "xml" => "xml",
        _ => "text",
    }
}

fn invalid_path() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "path",
        "Use a visible workspace-relative path without links or protected entries.",
    )
}

fn files_unavailable() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "workspace_files_unavailable",
        "Files are available only for the selected Managed Local workspace with Development access.",
        false,
    )
}

fn workspace_read_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "workspace_read_failed",
        "The workspace item could not be read. Refresh the workspace and try again.",
        true,
    )
}

fn preview_unavailable() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "file_preview_unavailable",
        "This file cannot be previewed as bounded UTF-8 text.",
        false,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::desktop_settings::validate_workspace;

    #[test]
    fn directory_listing_is_sorted_bounded_and_hides_sensitive_entries() {
        let root = tempdir().expect("root");
        fs::create_dir(root.path().join("src")).expect("src");
        fs::create_dir(root.path().join("target")).expect("target");
        fs::write(root.path().join("README.md"), "# Colossus\n").expect("readme");
        fs::write(root.path().join(".env"), "TOKEN=secret\n").expect("env");
        fs::write(root.path().join("Cargo.toml"), "[package]\n").expect("cargo");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            root.path().join("README.md"),
            root.path().join("linked-readme"),
        )
        .expect("symlink");

        let canonical = fs::canonicalize(root.path()).expect("canonical root");
        let listed = list_directory(&canonical, "").expect("list");

        assert_eq!(
            listed
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["src", "Cargo.toml", "README.md"]
        );
        assert!(listed.excluded_count >= 2);
        assert!(!listed.truncated);
    }

    #[test]
    fn file_preview_is_text_only_relative_and_language_aware() {
        let root = tempdir().expect("root");
        fs::create_dir(root.path().join("src")).expect("src");
        fs::write(
            root.path().join("src/lib.rs"),
            "pub fn ready() -> bool {\n    true\n}\n",
        )
        .expect("source");
        fs::write(root.path().join("binary.bin"), [0, 1, 2, 3]).expect("binary");

        let canonical = fs::canonicalize(root.path()).expect("canonical root");
        let preview = read_file(&canonical, "src/lib.rs").expect("preview");
        assert_eq!(preview.language, "rust");
        assert_eq!(preview.line_count, 4);
        assert!(preview.content.contains("pub fn ready"));
        assert!(read_file(&canonical, "../outside").is_err());
        assert!(read_file(&canonical, "binary.bin").is_err());
    }

    #[test]
    fn direct_access_to_protected_or_linked_files_fails_closed() {
        let root = tempdir().expect("root");
        fs::write(root.path().join(".env"), "TOKEN=secret\n").expect("env");
        fs::write(root.path().join("source.txt"), "safe\n").expect("source");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            root.path().join("source.txt"),
            root.path().join("source-link.txt"),
        )
        .expect("symlink");

        let canonical = fs::canonicalize(root.path()).expect("canonical root");
        assert!(read_file(&canonical, ".env").is_err());
        #[cfg(unix)]
        assert!(read_file(&canonical, "source-link.txt").is_err());
    }

    #[test]
    fn nested_credential_stores_are_excluded_from_listing_and_preview() {
        let root = tempdir().expect("root");
        for directory in [".docker", ".kube", ".config/gcloud"] {
            fs::create_dir_all(root.path().join(directory)).expect("credential directory");
        }
        fs::write(
            root.path().join(".docker/config.json"),
            "{\"auths\":{\"registry.example\":{\"auth\":\"secret\"}}}\n",
        )
        .expect("docker credentials");
        fs::write(
            root.path().join(".kube/config"),
            "users:\n- token: secret\n",
        )
        .expect("kube credentials");
        fs::write(
            root.path()
                .join(".config/gcloud/application_default_credentials.json"),
            "{\"client_secret\":\"secret\"}\n",
        )
        .expect("gcloud credentials");

        let canonical = fs::canonicalize(root.path()).expect("canonical root");
        let listed = list_directory(&canonical, "").expect("root listing");
        assert!(
            listed
                .entries
                .iter()
                .all(|entry| !matches!(entry.name.as_str(), ".docker" | ".kube"))
        );
        let config = list_directory(&canonical, ".config").expect("config listing");
        assert!(config.entries.iter().all(|entry| entry.name != "gcloud"));
        for protected in [
            ".docker/config.json",
            ".kube/config",
            ".config/gcloud/application_default_credentials.json",
        ] {
            assert!(read_file(&canonical, protected).is_err(), "{protected}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn authorization_binds_development_access_selected_target_and_workspace_id() {
        let root = tempdir().expect("root");
        let canonical = fs::canonicalize(root.path()).expect("canonical root");
        let workspace = validate_workspace(&canonical).expect("workspace");
        let mut settings = DesktopSettings {
            workspace: Some(workspace.clone()),
            selected_target_id: Some(MANAGED_TARGET_ID.into()),
            ..DesktopSettings::default()
        };

        assert_eq!(
            authorize_workspace(&settings, &workspace.id)
                .expect("authorized")
                .path,
            workspace.path
        );
        assert!(authorize_workspace(&settings, "other-workspace").is_err());
        settings.access_profile = AccessProfileSetting::Minimal;
        assert!(authorize_workspace(&settings, &workspace.id).is_err());
        settings.access_profile = AccessProfileSetting::Development;
        settings.selected_target_id = Some("external".into());
        assert!(authorize_workspace(&settings, &workspace.id).is_err());
    }
}
