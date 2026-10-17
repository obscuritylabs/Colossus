//! Private, fixed-vocabulary startup evidence for the signed Mac Debug fixture.
//! This is not a transport result, capability, or production diagnostic channel.

use colossus_home::ConfinedRoot;
use colossus_runtime::RuntimeError;
use colossus_worker::WorkerError;
use rustix::process::{Resource, Rlimit};
use std::{io::Write as _, path::Path};

const FILE: &str = "mac-browser-startup-category";
// The macOS native-profile snapshot retains one descriptor per ordinary file
// and directory up to its 8,192-entry bound. Leave room for bootstrap, IPC,
// directory iteration, and the runtime's other concurrently owned handles.
const NATIVE_PROFILE_CENSUS_ENTRY_BOUND: u64 = 8_192;
const PROCESS_DESCRIPTOR_HEADROOM: u64 = 1_024;
const REQUIRED_NOFILE_SOFT_LIMIT: u64 =
    NATIVE_PROFILE_CENSUS_ENTRY_BOUND + PROCESS_DESCRIPTOR_HEADROOM;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Category {
    NativeProfileFdBudget,
    NativeProfileProtection,
    ExplicitHome,
    HomeWorkspaceStorage,
    DevelopmentCredentialBoundary,
    PluginTrust,
    WorkspaceConfiguration,
    OtherRuntimeConfiguration,
    OtherWorkerOpenFailure,
}

impl Category {
    const fn token(self) -> &'static str {
        match self {
            Self::NativeProfileFdBudget => "native_profile_fd_budget\n",
            Self::NativeProfileProtection => "native_profile_protection\n",
            Self::ExplicitHome => "explicit_home\n",
            Self::HomeWorkspaceStorage => "home_workspace_storage\n",
            Self::DevelopmentCredentialBoundary => "development_credential_boundary\n",
            Self::PluginTrust => "plugin_trust\n",
            Self::WorkspaceConfiguration => "workspace_configuration\n",
            Self::OtherRuntimeConfiguration => "other_runtime_configuration\n",
            Self::OtherWorkerOpenFailure => "other_worker_open_failure\n",
        }
    }
}

fn limit_to_install(current: Rlimit) -> Result<Option<Rlimit>, ()> {
    if current
        .current
        .is_none_or(|soft| soft >= REQUIRED_NOFILE_SOFT_LIMIT)
    {
        return Ok(None);
    }
    if current
        .maximum
        .is_some_and(|hard| hard < REQUIRED_NOFILE_SOFT_LIMIT)
    {
        return Err(());
    }
    Ok(Some(Rlimit {
        current: Some(REQUIRED_NOFILE_SOFT_LIMIT),
        maximum: current.maximum,
    }))
}

/// Bound the process-local descriptor budget for the retained native cache
/// census. The hard limit and all system/launchd settings remain unchanged.
pub(super) fn prepare_native_profile_fd_budget(root: &ConfinedRoot) -> Result<(), ()> {
    let original = rustix::process::getrlimit(Resource::Nofile);
    let result = (|| {
        if let Some(target) = limit_to_install(original)? {
            rustix::process::setrlimit(Resource::Nofile, target).map_err(|_| ())?;
        }
        let actual = rustix::process::getrlimit(Resource::Nofile);
        if actual.maximum != original.maximum || limit_to_install(actual)?.is_some() {
            return Err(());
        }
        Ok(())
    })();
    if result.is_err() {
        record_category(root, Category::NativeProfileFdBudget);
    }
    result
}

fn classify(error: &WorkerError) -> Category {
    let WorkerError::Runtime(RuntimeError::Config(message)) = error else {
        return Category::OtherWorkerOpenFailure;
    };
    if message == "native browser profile confinement is invalid" {
        Category::NativeProfileProtection
    } else if message.starts_with("the explicit Colossus home") {
        Category::ExplicitHome
    } else if message.starts_with("home-workspace ")
        || message.starts_with("storage.location home_workspace")
    {
        Category::HomeWorkspaceStorage
    } else if message.starts_with("development credential")
        || message.starts_with("explicit development credential")
    {
        Category::DevelopmentCredentialBoundary
    } else if message.starts_with("plugins.trustProfiles.obscuritylabs") {
        Category::PluginTrust
    } else if message.starts_with("workspace is not a directory") {
        Category::WorkspaceConfiguration
    } else {
        Category::OtherRuntimeConfiguration
    }
}

/// Record one bounded category only after the private root was descriptor-bound.
/// An existing leaf is never overwritten, so same-user replacement cannot turn
/// this diagnostic into a write selector or an authority assertion.
pub(super) fn record(root: &ConfinedRoot, error: &WorkerError) {
    record_category(root, classify(error));
}

fn record_category(root: &ConfinedRoot, category: Category) {
    let Ok(file) = root.open_file(Path::new(FILE)) else {
        return;
    };
    if !file.was_created() {
        return;
    }
    let mut output = file.file();
    if output.write_all(category.token().as_bytes()).is_ok() {
        let _ = output.sync_all();
        let _ = file.revalidate(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};

    #[test]
    fn nofile_budget_never_raises_hard_and_fails_closed_when_hard_is_too_low() {
        let enough = REQUIRED_NOFILE_SOFT_LIMIT;
        assert_eq!(
            limit_to_install(Rlimit {
                current: None,
                maximum: None
            }),
            Ok(None)
        );
        assert_eq!(
            limit_to_install(Rlimit {
                current: Some(enough),
                maximum: Some(enough),
            }),
            Ok(None)
        );
        assert_eq!(
            limit_to_install(Rlimit {
                current: Some(256),
                maximum: Some(enough + 1),
            }),
            Ok(Some(Rlimit {
                current: Some(enough),
                maximum: Some(enough + 1),
            }))
        );
        assert_eq!(
            limit_to_install(Rlimit {
                current: Some(256),
                maximum: None,
            }),
            Ok(Some(Rlimit {
                current: Some(enough),
                maximum: None,
            }))
        );
        assert_eq!(
            limit_to_install(Rlimit {
                current: Some(256),
                maximum: Some(enough - 1),
            }),
            Err(())
        );
        assert_eq!(
            Category::NativeProfileFdBudget.token(),
            "native_profile_fd_budget\n"
        );
    }

    #[test]
    fn startup_categories_are_closed_vocabulary_and_never_copy_error_content() {
        let examples = [
            (
                "native browser profile confinement is invalid",
                Category::NativeProfileProtection,
            ),
            (
                "the explicit Colossus home is unsafe: /private/secret",
                Category::ExplicitHome,
            ),
            (
                "home-workspace state path is unsafe: /private/secret",
                Category::HomeWorkspaceStorage,
            ),
            (
                "explicit development credential authority is invalid or inactive",
                Category::DevelopmentCredentialBoundary,
            ),
            (
                "plugins.trustProfiles.obscuritylabs is a reserved built-in signing identity",
                Category::PluginTrust,
            ),
            (
                "workspace is not a directory: /private/secret",
                Category::WorkspaceConfiguration,
            ),
            (
                "unclassified secret /private/secret",
                Category::OtherRuntimeConfiguration,
            ),
        ];
        for (message, expected) in examples {
            let error = WorkerError::Runtime(RuntimeError::Config(message.into()));
            let category = classify(&error);
            assert_eq!(category, expected);
            assert!(!category.token().contains("secret"));
            assert!(!category.token().contains("private"));
        }
    }

    #[test]
    fn startup_category_is_create_new_private_and_does_not_follow_replacements() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = ConfinedRoot::bind(directory.path().canonicalize().unwrap()).unwrap();
        let first = WorkerError::Runtime(RuntimeError::Config(
            "native browser profile confinement is invalid".into(),
        ));
        record(&root, &first);
        let category = directory.path().join(FILE);
        assert_eq!(
            fs::read_to_string(&category).unwrap(),
            "native_profile_protection\n"
        );
        let metadata = fs::symlink_metadata(&category).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(metadata.nlink(), 1);

        record(
            &root,
            &WorkerError::Runtime(RuntimeError::Config("other /private/secret".into())),
        );
        assert_eq!(
            fs::read_to_string(&category).unwrap(),
            "native_profile_protection\n"
        );

        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::remove_file(&category).unwrap();
        symlink(outside.path(), &category).unwrap();
        record(&root, &first);
        assert_eq!(fs::read(outside.path()).unwrap(), b"");
    }
}
