use super::*;
use std::{collections::BTreeMap, fs, path::Path};

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("staged directory");
    for entry in fs::read_dir(source).expect("source directory") {
        let entry = entry.expect("source entry");
        if entry.file_type().expect("source type").is_dir() {
            copy_tree(&entry.path(), &destination.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), destination.join(entry.file_name())).expect("staged file");
        }
    }
}

fn tree_bytes(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(root).expect("tree directory") {
        let entry = entry.expect("tree entry");
        if entry.file_type().expect("tree type").is_dir() {
            for (path, bytes) in tree_bytes(&entry.path()) {
                result.insert(Path::new(&entry.file_name()).join(path), bytes);
            }
        } else {
            result.insert(
                entry.file_name().into(),
                fs::read(entry.path()).expect("file"),
            );
        }
    }
    result
}

#[test]
fn embedded_core_matches_directory_packaging_and_contains_complete_documentation() {
    let embedded = core_artifact().expect("embedded artifact");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary = tempfile::tempdir().expect("temporary");
    let staged = temporary.path().join("staged");
    copy_tree(&repository.join("bundled-plugins/colossus"), &staged);
    copy_tree(
        &repository.join("docs"),
        &staged.join("skills/help/references/docs"),
    );
    let directory = colossus_plugins::build_plugin_artifact(&staged).expect("directory artifact");
    assert_eq!(embedded.manifest_digest, directory.manifest_digest);
    assert_eq!(embedded.layer, directory.layer);
    let root = temporary.path().join("content");
    colossus_plugins::extract_plugin_artifact(&embedded, &root).expect("extract");
    let expected_docs = tree_bytes(&repository.join("docs"));
    let bundled_docs = tree_bytes(&root.join("skills/help/references/docs"));
    assert_eq!(
        bundled_docs.keys().collect::<Vec<_>>(),
        expected_docs.keys().collect::<Vec<_>>(),
        "the snapshot must contain exactly the canonical documentation tree"
    );
    for (path, bytes) in &expected_docs {
        assert!(
            bundled_docs.get(path) == Some(bytes),
            "documentation changed during packaging: {}",
            path.display()
        );
    }
    let record = colossus_plugins::load_plugin(&root).expect("load");
    assert!(
        record
            .icon_data_url
            .as_deref()
            .is_some_and(|icon| icon.starts_with("data:image/png;base64,"))
    );
    assert!(record.diagnostics.is_empty(), "{:?}", record.diagnostics);
    assert_eq!(
        record
            .skills
            .iter()
            .map(|skill| skill.id.as_str())
            .collect::<Vec<_>>(),
        [
            "colossus/coding",
            "colossus/help",
            "colossus/offline-dev",
            "colossus/plugin-authoring",
            "colossus/security-review"
        ]
    );

    // Read through the same bounded resource API used by installed clients. The staged
    // source is disposable; the extracted artifact owns all documentation bytes.
    fs::remove_dir_all(staged).expect("remove staging source");
    let help = record
        .skills
        .iter()
        .find(|skill| skill.id == "colossus/help")
        .expect("help");
    let resources = colossus_plugins::list_resources(help).expect("help resources");
    assert_eq!(resources.len(), expected_docs.len());
    for page in [
        "index.md",
        "admin/troubleshooting.md",
        "reference/configuration.md",
    ] {
        let resource = colossus_plugins::read_resource(help, &format!("references/docs/{page}"))
            .expect("read bundled documentation");
        assert_eq!(
            resource.content,
            fs::read_to_string(repository.join("docs").join(page)).expect("canonical page")
        );
    }
    assert!(resources.iter().any(|resource| resource.path
        == "references/docs/assets/screenshots/tui-offline-session.png"
        && !resource.text));

    // The assembled tree remains an ordinary portable plugin, including docs changes.
    let path = root.join("skills/help/references/docs/admin/troubleshooting.md");
    fs::write(path, b"Changed documentation\n").expect("change staged documentation");
    let changed = colossus_plugins::build_plugin_artifact(&root).expect("changed artifact");
    assert_ne!(embedded.manifest_digest, changed.manifest_digest);
}

#[test]
fn authoring_templates_are_valid_portable_plugins() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bundled-plugins/colossus/skills/plugin-authoring/assets/templates");
    for name in ["skills-only", "stdio-mcp", "http-mcp"] {
        let source = root.join(name);
        let record = colossus_plugins::load_plugin(&source).expect("template plugin");
        assert!(
            record.diagnostics.is_empty(),
            "{name}: {:?}",
            record.diagnostics
        );
        let artifact = colossus_plugins::build_plugin_artifact(&source).expect("template artifact");
        let temp = tempfile::tempdir().expect("temporary");
        let extracted = temp.path().join("plugin");
        colossus_plugins::extract_plugin_artifact(&artifact, &extracted).expect("extract template");
        assert!(
            colossus_plugins::load_plugin(&extracted)
                .expect("load template")
                .diagnostics
                .is_empty()
        );
    }
}
