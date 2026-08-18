#[cfg(unix)]
use super::MAX_SKILL_ROOTS;
use super::{
    FilesystemSkillRepository, SkillAuthoringService, SkillComposer, SkillResourceService,
    SkillRoot, content_hash, split_frontmatter,
};
use colossus_contracts::ToolSpec;
use colossus_ports::SkillRepository;
#[cfg(unix)]
use colossus_ports::StoreError;
use std::{fs, sync::Arc};
use tempfile::tempdir;

#[cfg(unix)]
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

fn write_skill(root: &std::path::Path, name: &str, required_tools: &[&str]) {
    fs::create_dir_all(root.join("references")).expect("directory");
    fs::write(root.join("SKILL.md"), format!("Instructions for {name}.")).expect("skill");
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": name,
            "version": "1.0.0",
            "description": format!("{name} skill"),
            "triggers": [name],
            "required_tools": required_tools,
            "permissions": [],
            "offline_compatible": true
        }))
        .expect("JSON"),
    )
    .expect("manifest");
    fs::write(root.join("references/guide.md"), "# Guide\n").expect("resource");
}

#[cfg(unix)]
fn create_fifo(path: &Path) {
    nix::unistd::mkfifo(
        path,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .expect("FIFO");
}

#[cfg(unix)]
fn assert_fifo_operation_does_not_block<T>(
    fifo: PathBuf,
    operation: impl FnOnce() -> T + Send + 'static,
) -> T
where
    T: Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = operation();
        sender.send(result).expect("test receiver");
    });
    let result = match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let writer = nix::fcntl::open(
                &fifo,
                nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_NONBLOCK,
                nix::sys::stat::Mode::empty(),
            )
            .expect("unblock FIFO reader");
            let _ = receiver.recv_timeout(Duration::from_secs(2));
            drop(writer);
            worker.join().expect("FIFO worker");
            panic!("skill repository blocked while opening a FIFO")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            worker.join().expect("FIFO worker");
            unreachable!("worker exited without returning a result")
        }
    };
    worker.join().expect("FIFO worker");
    result
}

#[test]
fn precedence_composition_and_required_tools_are_deterministic() {
    let directory = tempdir().expect("tempdir");
    let bundled = directory.path().join("bundled");
    let user = directory.path().join("user");
    write_skill(&bundled.join("alpha"), "alpha", &["echo"]);
    write_skill(&user.join("alpha"), "alpha", &[]);
    write_skill(&user.join("beta"), "beta", &[]);
    let repository: Arc<dyn SkillRepository> = Arc::new(
        FilesystemSkillRepository::new(
            vec![
                SkillRoot {
                    path: bundled,
                    label: "bundled".into(),
                },
                SkillRoot {
                    path: user,
                    label: "user".into(),
                },
            ],
            false,
            Vec::new(),
        )
        .expect("repository"),
    );
    let skills = repository.list_skills().expect("skills");
    assert_eq!(
        skills
            .iter()
            .map(|skill| skill.manifest.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
    assert_eq!(skills[0].source, "bundled:alpha");
    assert_eq!(
        repository.duplicate_names().expect("duplicates")[0].selected_source,
        "bundled:alpha"
    );
    let composer = SkillComposer::new(repository);
    assert!(
        composer
            .compose("Base", "@alpha", &[], &[], true, &[])
            .is_err()
    );
    let composition = composer
        .compose(
            "Base",
            "@skill:alpha then @beta",
            &[],
            &[],
            true,
            &[ToolSpec {
                name: "echo".into(),
                description: "Echo".into(),
                input_schema: serde_json::json!({"type":"object"}),
                effect_action: None,
                capability: None,
                max_output_bytes: 1_024,
            }],
        )
        .expect("composition");
    assert_eq!(
        composition
            .active_skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
    assert!(composition.instructions.contains("Instructions for alpha"));
}

#[test]
fn resources_are_active_scoped_bounded_text_only_and_symlink_safe() {
    let directory = tempdir().expect("tempdir");
    let root = directory.path().join("skills/demo");
    write_skill(&root, "demo", &[]);
    fs::write(root.join("references/blob.bin"), b"a\0b").expect("blob");
    fs::write(root.join("references/huge.txt"), vec![b'x'; 64_001]).expect("huge");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", root.join("references/escape.txt")).expect("symlink");
    let repository: Arc<dyn SkillRepository> = Arc::new(
        FilesystemSkillRepository::new(
            vec![SkillRoot {
                path: directory.path().join("skills"),
                label: "test".into(),
            }],
            false,
            Vec::new(),
        )
        .expect("repository"),
    );
    let service = SkillResourceService::new(repository);
    assert!(service.list_resources_inner("demo", &[]).is_err());
    let active = vec!["demo".into()];
    let resources = service
        .list_resources_inner("demo", &active)
        .expect("resources");
    assert!(
        resources
            .iter()
            .any(|entry| entry.path == "references/guide.md")
    );
    assert!(
        !resources
            .iter()
            .any(|entry| entry.path.ends_with("escape.txt"))
    );
    assert_eq!(
        service
            .read_resource_inner("demo", "references/guide.md", &active)
            .expect("read")
            .content,
        "# Guide\n"
    );
    assert!(
        service
            .read_resource_inner("demo", "../outside", &active)
            .is_err()
    );
    assert!(
        service
            .read_resource_inner("demo", "references/blob.bin", &active)
            .is_err()
    );
    assert!(
        service
            .read_resource_inner("demo", "references/huge.txt", &active)
            .is_err()
    );
    assert_eq!(content_hash(b"hello").len(), 64);
}

#[cfg(unix)]
#[test]
fn workspace_bound_skills_and_resources_ignore_an_aba_path_replacement() {
    let directory = tempdir().expect("tempdir");
    let parent = fs::canonicalize(directory.path()).expect("canonical parent");
    let workspace = parent.join("workspace");
    let moved = parent.join("workspace-moved");
    let replacement = parent.join("workspace-replacement");
    fs::create_dir(&workspace).expect("workspace");
    fs::create_dir(&replacement).expect("replacement workspace");
    write_skill(&workspace.join("skills/demo"), "demo", &[]);
    write_skill(&replacement.join("skills/demo"), "demo", &[]);
    fs::write(
        replacement.join("skills/demo/SKILL.md"),
        "Malicious replacement instructions.",
    )
    .expect("replacement instructions");
    fs::write(
        replacement.join("skills/demo/references/guide.md"),
        "malicious replacement resource\n",
    )
    .expect("replacement resource");

    let repository: Arc<dyn SkillRepository> = Arc::new(
        FilesystemSkillRepository::new_workspace_bound(
            fs::File::open(&workspace).expect("workspace descriptor"),
            &workspace,
            vec![SkillRoot {
                path: workspace.join("skills"),
                label: "workspace".into(),
            }],
            false,
            Vec::new(),
        )
        .expect("bound repository"),
    );

    fs::rename(&workspace, &moved).expect("move selected workspace");
    fs::rename(&replacement, &workspace).expect("install replacement at selected path");

    let composition = SkillComposer::new(Arc::clone(&repository))
        .compose("Base", "@skill:demo", &["demo".into()], &[], true, &[])
        .expect("compose through retained workspace");
    let resources = SkillResourceService::new(repository);
    let active = vec!["demo".into()];
    let listing = resources
        .list_resources_inner("demo", &active)
        .expect("list through retained skill directory");
    let resource = resources
        .read_resource_inner("demo", "references/guide.md", &active)
        .expect("read through retained skill directory");

    fs::rename(&workspace, &replacement).expect("remove replacement");
    fs::rename(&moved, &workspace).expect("restore selected workspace");

    assert!(composition.instructions.contains("Instructions for demo."));
    assert!(!composition.instructions.contains("Malicious replacement"));
    assert!(
        listing
            .iter()
            .any(|entry| entry.path == "references/guide.md")
    );
    assert_eq!(resource.content, "# Guide\n");
    assert!(!resource.content.contains("malicious"));
}

#[cfg(unix)]
#[test]
fn workspace_bound_missing_roots_are_discovered_later_from_the_retained_directory() {
    let directory = tempdir().expect("tempdir");
    let workspace = fs::canonicalize(directory.path()).expect("canonical workspace");
    let repository = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: workspace.join("skills-created-later"),
            label: "workspace".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("bound repository");
    assert!(repository.list_skills().expect("initial list").is_empty());

    write_skill(&workspace.join("skills-created-later/demo"), "demo", &[]);
    assert_eq!(
        repository
            .list_skills()
            .expect("late root list")
            .into_iter()
            .map(|skill| skill.manifest.name)
            .collect::<Vec<_>>(),
        vec!["demo"]
    );
}

#[cfg(unix)]
#[test]
fn workspace_bound_rejects_oversized_root_sets_before_binding_any_root() {
    let directory = tempdir().expect("tempdir");
    let parent = fs::canonicalize(directory.path()).expect("canonical parent");
    let workspace = parent.join("workspace");
    let outside = parent.join("outside");
    fs::create_dir(&workspace).expect("workspace");
    fs::create_dir(&outside).expect("outside");
    std::os::unix::fs::symlink(&outside, parent.join("must-not-open")).expect("external symlink");
    let mut roots = (0..MAX_SKILL_ROOTS)
        .map(|index| SkillRoot {
            path: workspace.join(format!("skills-{index}")),
            label: format!("workspace-{index}"),
        })
        .collect::<Vec<_>>();
    roots.insert(
        0,
        SkillRoot {
            // Binding this first root would fail at its symlink component. The
            // aggregate error below therefore proves validation precedes traversal.
            path: parent.join("must-not-open/skills"),
            label: "must-not-open".into(),
        },
    );

    let error = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        roots,
        false,
        Vec::new(),
    )
    .err()
    .expect("oversized roots must fail before binding");
    let StoreError::Adapter(message) = error else {
        panic!("expected aggregate root validation error, got {error}")
    };
    assert_eq!(
        message,
        format!("skill roots exceed the aggregate limit of {MAX_SKILL_ROOTS}")
    );
}

#[cfg(unix)]
#[test]
fn workspace_bound_external_roots_retain_the_opened_object() {
    let directory = tempdir().expect("tempdir");
    let parent = fs::canonicalize(directory.path()).expect("canonical parent");
    let workspace = parent.join("workspace");
    let outside = parent.join("outside");
    let moved = parent.join("outside-moved");
    fs::create_dir(&workspace).expect("workspace");
    fs::create_dir(&outside).expect("outside");
    write_skill(&outside.join("skills/demo"), "demo", &[]);

    let repository = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: outside.join("skills"),
            label: "outside".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("external root capability");

    fs::rename(&outside, &moved).expect("move external root");
    write_skill(&outside.join("skills/demo"), "demo", &[]);
    fs::write(
        outside.join("skills/demo/SKILL.md"),
        "Malicious external replacement instructions.",
    )
    .expect("replacement instructions");
    fs::write(
        outside.join("skills/demo/references/guide.md"),
        "malicious external replacement resource\n",
    )
    .expect("replacement resource");

    let skills = repository.list_skills().expect("retained external skills");
    let resource = repository
        .read_skill_resource("demo", "references/guide.md")
        .expect("retained external resource");

    fs::remove_dir_all(&outside).expect("remove external replacement");
    fs::rename(&moved, &outside).expect("restore external root");

    assert_eq!(skills.len(), 1);
    assert!(skills[0].instructions.contains("Instructions for demo."));
    assert!(!skills[0].instructions.contains("Malicious"));
    assert_eq!(resource.content, "# Guide\n");
}

#[cfg(unix)]
#[test]
fn workspace_bound_missing_external_roots_require_a_private_retained_ancestor() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempdir().expect("tempdir");
    let parent = fs::canonicalize(directory.path()).expect("canonical parent");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
        .expect("private app-support ancestor");
    let workspace = parent.join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    let repository = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: parent.join("app-support/skills"),
            label: "app-support".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("missing root beneath private ancestor");
    assert!(repository.list_skills().expect("initial list").is_empty());
    write_skill(&parent.join("app-support/skills/demo"), "demo", &[]);
    assert_eq!(repository.list_skills().expect("late list").len(), 1);

    let shared = parent.join("shared");
    fs::create_dir(&shared).expect("shared ancestor");
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).expect("shared permissions");
    let error = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: shared.join("missing/skills"),
            label: "unsafe-missing".into(),
        }],
        false,
        Vec::new(),
    )
    .err()
    .expect("missing external root beneath shared ancestor must fail");
    assert!(matches!(error, StoreError::Adapter(_)));
}

#[cfg(unix)]
#[test]
fn workspace_bound_roots_fail_closed_at_symlink_components() {
    let directory = tempdir().expect("tempdir");
    let parent = fs::canonicalize(directory.path()).expect("canonical parent");
    let workspace = parent.join("workspace");
    let outside = parent.join("outside");
    fs::create_dir(&workspace).expect("workspace");
    fs::create_dir(&outside).expect("outside");
    write_skill(&outside.join("skills/demo"), "demo", &[]);

    std::os::unix::fs::symlink(&outside, workspace.join("linked-root")).expect("root symlink");
    let linked_repository = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: workspace.join("linked-root/skills"),
            label: "linked".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("lexically contained root");
    assert!(linked_repository.list_skills().is_err());

    write_skill(&workspace.join("skills/demo"), "demo", &[]);
    fs::remove_dir_all(workspace.join("skills/demo/references")).expect("remove references");
    std::os::unix::fs::symlink(
        outside.join("skills/demo/references"),
        workspace.join("skills/demo/references"),
    )
    .expect("resource symlink");
    let resource_repository = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: workspace.join("skills"),
            label: "workspace".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("resource repository");
    assert!(resource_repository.list_skill_resources("demo").is_err());
    assert!(
        resource_repository
            .read_skill_resource("demo", "references/guide.md")
            .is_err()
    );

    std::os::unix::fs::symlink(&outside, parent.join("external-link")).expect("external symlink");
    let external_error = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: parent.join("external-link/skills"),
            label: "external-link".into(),
        }],
        false,
        Vec::new(),
    )
    .err()
    .expect("external symlink component must fail");
    assert!(matches!(external_error, StoreError::Adapter(_)));
}

#[cfg(unix)]
#[test]
fn workspace_bound_special_files_fail_without_blocking() {
    let directory = tempdir().expect("tempdir");
    let workspace = fs::canonicalize(directory.path()).expect("canonical workspace");

    let instruction_root = workspace.join("instruction-fifo/demo");
    fs::create_dir_all(&instruction_root).expect("instruction root");
    let instruction_fifo = instruction_root.join("SKILL.md");
    create_fifo(&instruction_fifo);
    let instruction_repository = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: workspace.join("instruction-fifo"),
            label: "instruction-fifo".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("instruction repository");
    let instruction_result = assert_fifo_operation_does_not_block(instruction_fifo, move || {
        instruction_repository.list_skills()
    });
    assert!(instruction_result.is_err());

    let manifest_root = workspace.join("manifest-fifo/demo");
    fs::create_dir_all(&manifest_root).expect("manifest root");
    fs::write(
        manifest_root.join("SKILL.md"),
        "---\nname: demo\ndescription: Demo\n---\nTrusted instructions.\n",
    )
    .expect("instructions");
    let manifest_fifo = manifest_root.join("manifest.json");
    create_fifo(&manifest_fifo);
    let manifest_repository = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: workspace.join("manifest-fifo"),
            label: "manifest-fifo".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("manifest repository");
    let manifest_result = assert_fifo_operation_does_not_block(manifest_fifo, move || {
        manifest_repository.list_skills()
    });
    assert!(manifest_result.is_err());

    let resource_root = workspace.join("resource-fifo/demo");
    write_skill(&resource_root, "demo", &[]);
    let resource_fifo = resource_root.join("references/pipe.txt");
    create_fifo(&resource_fifo);
    let resource_repository = FilesystemSkillRepository::new_workspace_bound(
        fs::File::open(&workspace).expect("workspace descriptor"),
        &workspace,
        vec![SkillRoot {
            path: workspace.join("resource-fifo"),
            label: "resource-fifo".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("resource repository");
    let resource_result = assert_fifo_operation_does_not_block(resource_fifo, move || {
        resource_repository.read_skill_resource("demo", "references/pipe.txt")
    });
    assert!(resource_result.is_err());
}

#[test]
fn authoring_scaffold_read_and_optimistic_write_are_validated() {
    let directory = tempdir().expect("tempdir");
    let user = directory.path().join("user");
    let service = SkillAuthoringService::new(
        user.clone(),
        directory.path().canonicalize().expect("workspace"),
    )
    .expect("service");
    let scaffold = service
        .scaffold_inner(
            "demo",
            "Demo skill",
            "Use bounded data-only instructions.",
            &["references".into()],
        )
        .expect("scaffold");
    assert_eq!(scaffold.name, "demo");
    let current = service
        .read_installed_inner("demo", "SKILL.md")
        .expect("read");
    assert!(
        service
            .write_installed_inner("demo", "SKILL.md", "Changed", None)
            .is_err()
    );
    let written = service
        .write_installed_inner(
            "demo",
            "SKILL.md",
            "Changed instructions.",
            Some(&current.sha256),
        )
        .expect("write");
    assert_eq!(
        written.previous_sha256.as_deref(),
        Some(current.sha256.as_str())
    );
    assert!(
        service
            .write_installed_inner("demo", "SKILL.md", "Stale write.", Some(&current.sha256),)
            .is_err()
    );
    service
        .write_installed_inner("demo", "references/guide.md", "# Guide\n", None)
        .expect("new resource");
    let validation = service.validate_installed_inner("demo").expect("valid");
    assert_eq!(validation.name, "demo");
    assert_eq!(validation.file_count, 3);
    assert!(
        service
            .write_installed_inner("demo", "outside.md", "denied", None)
            .is_err()
    );
}

#[test]
fn local_install_is_workspace_contained_non_overwriting_and_symlink_free() {
    let directory = tempdir().expect("tempdir");
    let source = directory.path().join("sources/local");
    write_skill(&source, "local", &[]);
    let service = SkillAuthoringService::new(
        directory.path().join("user"),
        directory.path().canonicalize().expect("workspace"),
    )
    .expect("service");
    let validated = service
        .validate_local_inner(std::path::Path::new("sources/local"))
        .expect("validate");
    let installed = service
        .install_local_inner(std::path::Path::new("sources/local"))
        .expect("install");
    assert_eq!(installed.content_sha256, validated.content_sha256);
    assert!(
        service
            .install_local_inner(std::path::Path::new("sources/local"))
            .is_err()
    );
    assert!(
        service
            .validate_local_inner(std::path::Path::new("../escape"))
            .is_err()
    );

    #[cfg(unix)]
    {
        let unsafe_source = directory.path().join("sources/unsafe");
        write_skill(&unsafe_source, "unsafe", &[]);
        std::os::unix::fs::symlink("/etc/passwd", unsafe_source.join("references/escape.txt"))
            .expect("symlink");
        assert!(service.validate_local_inner(&unsafe_source).is_err());
    }
}

#[test]
fn protocol_skill_frontmatter_is_line_ending_independent() {
    let directory = tempdir().expect("tempdir");
    for (name, newline) in [("frontmatter-lf", "\n"), ("frontmatter-crlf", "\r\n")] {
        let root = directory.path().join("skills").join(name);
        fs::create_dir_all(&root).expect("directory");
        let content = format!(
            "---{newline}name: {name}{newline}description: Protocol skill{newline}---{newline}Use it safely.{newline}"
        );
        fs::write(root.join("skill.md"), content).expect("skill");
    }
    let repository = FilesystemSkillRepository::new(
        vec![SkillRoot {
            path: directory.path().join("skills"),
            label: "test".into(),
        }],
        false,
        Vec::new(),
    )
    .expect("repository");
    let lf = repository
        .get_skill("frontmatter-lf")
        .expect("get LF")
        .expect("LF skill");
    let crlf = repository
        .get_skill("frontmatter-crlf")
        .expect("get CRLF")
        .expect("CRLF skill");
    assert_eq!(lf.manifest.version, "0.1.0");
    assert_eq!(crlf.manifest.version, "0.1.0");
    assert_eq!(lf.manifest.description, crlf.manifest.description);
    assert_eq!(lf.instructions, "Use it safely.\n");
    assert_eq!(crlf.instructions, "Use it safely.\r\n");

    assert!(
        split_frontmatter("---\r\nname: malformed\r\n---suffix\r\nBody\r\n").is_err(),
        "a look-alike closing marker must not terminate frontmatter"
    );
}
