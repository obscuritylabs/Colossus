use super::{
    FilesystemSkillRepository, SkillAuthoringService, SkillComposer, SkillResourceService,
    SkillRoot, content_hash, split_frontmatter,
};
use colossus_contracts::ToolSpec;
use colossus_ports::SkillRepository;
use std::{fs, sync::Arc};
use tempfile::tempdir;

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
