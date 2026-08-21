//! Executable examples published in the documentation.

use colossus_access::builtin_action_descriptors;
use colossus_runtime::RuntimeConfig;
use colossus_tools::builtin_specs;
use colossus_workflow::validate_definition;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repository root")
}

fn read(relative: &str) -> String {
    fs::read_to_string(repository_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn marked_yaml<'a>(document: &'a str, marker: &str) -> &'a str {
    let start = format!("<!-- {marker}:start -->");
    let end = format!("<!-- {marker}:end -->");
    let (_, after_start) = document.split_once(&start).expect("start marker");
    let (section, _) = after_start.split_once(&end).expect("end marker");
    let (_, yaml) = section.split_once("```yaml\n").expect("YAML fence");
    yaml.split_once("\n```").expect("closing fence").0
}

fn yaml_files(relative_directory: &str) -> Vec<PathBuf> {
    let directory = repository_root().join(relative_directory);
    let mut paths = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("YAML example entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("yaml"))
        .collect::<Vec<_>>();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "{relative_directory} must contain at least one YAML example"
    );
    paths
}

#[test]
fn published_configuration_baseline_matches_runtime_defaults() {
    let configuration = read("docs/reference/configuration.md");
    let documented_yaml = marked_yaml(&configuration, "rust-config-example");
    RuntimeConfig::from_yaml(documented_yaml).expect("documented configuration must parse");

    let documented: serde_json::Value =
        serde_saphyr::from_str(documented_yaml).expect("documented configuration value");
    let resolved_defaults = RuntimeConfig::from_yaml(
        "schemaVersion: 2\nstorage:\n  location: home_workspace\n  path: state.redb\n",
    )
    .expect("minimal global configuration")
    .to_resolved_yaml()
    .expect("resolved default configuration");
    let resolved_defaults: serde_json::Value =
        serde_saphyr::from_str(&resolved_defaults).expect("resolved default value");

    assert_eq!(
        documented, resolved_defaults,
        "the documented baseline must match `config show` for a minimal global configuration"
    );
}

#[test]
fn published_workflow_examples_are_accepted_by_the_runtime_parser() {
    for (page, marker) in [
        (
            "docs/extend/workflows/authoring.md",
            "rust-workflow-example",
        ),
        ("docs/reference/workflow-schema.md", "rust-workflow-example"),
    ] {
        let document = read(page);
        validate_definition(marked_yaml(&document, marker))
            .unwrap_or_else(|error| panic!("validate workflow in {page}: {error}"));
    }
}

fn merge_yaml_value(target: &mut serde_json::Value, overlay: serde_json::Value) {
    match (target, overlay) {
        (serde_json::Value::Object(target), serde_json::Value::Object(overlay)) => {
            for (key, value) in overlay {
                if let Some(current) = target.get_mut(&key) {
                    merge_yaml_value(current, value);
                } else {
                    target.insert(key, value);
                }
            }
        }
        (target, overlay) => *target = overlay,
    }
}

#[test]
fn provider_guide_overlays_are_accepted_by_the_runtime_parser() {
    let baseline = read("docs/reference/configuration.md");
    let baseline = marked_yaml(&baseline, "rust-config-example");

    for page in [
        "docs/use/providers/codex-chatgpt.md",
        "docs/use/providers/openai-api.md",
        "docs/use/providers/openrouter.md",
        "docs/use/providers/local-models.md",
        "docs/use/providers/openai-compatible.md",
    ] {
        let document = read(page);
        let mut configuration: serde_json::Value =
            serde_saphyr::from_str(baseline).expect("baseline configuration YAML");
        let overlay: serde_json::Value =
            serde_saphyr::from_str(marked_yaml(&document, "provider-guide-config"))
                .unwrap_or_else(|error| panic!("parse {page} provider overlay: {error}"));
        let overlay = overlay
            .as_object()
            .unwrap_or_else(|| panic!("{page} provider overlay must be a mapping"))
            .clone();
        let configuration_root = configuration
            .as_object_mut()
            .expect("baseline configuration mapping");

        for (key, value) in overlay {
            if key == "sandbox" {
                merge_yaml_value(
                    configuration_root
                        .get_mut("sandbox")
                        .expect("baseline sandbox configuration"),
                    value,
                );
            } else {
                configuration_root.insert(key, value);
            }
        }

        let yaml = serde_saphyr::to_string(&configuration).expect("provider guide YAML");
        RuntimeConfig::from_yaml(&yaml)
            .unwrap_or_else(|error| panic!("{page} provider overlay must parse: {error}"));
    }
}

#[test]
fn repository_workflow_examples_are_accepted_by_the_runtime_parser() {
    let mut identities = BTreeSet::new();
    for path in yaml_files("examples/workflows") {
        let yaml = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let validated = validate_definition(&yaml)
            .unwrap_or_else(|error| panic!("validate {}: {error}", path.display()));
        let identity = (
            validated.definition.metadata.name,
            validated.definition.metadata.version,
        );
        assert!(
            identities.insert(identity.clone()),
            "duplicate example workflow identity {}:{}",
            identity.0,
            identity.1
        );
    }
}

#[test]
fn tools_and_actions_reference_covers_the_executable_catalog() {
    let reference = read("docs/reference/tools-actions.md");
    for specification in builtin_specs() {
        assert!(
            reference.contains(&format!("`{}`", specification.name)),
            "tools reference omits built-in tool {}",
            specification.name
        );
    }
    for descriptor in builtin_action_descriptors() {
        assert!(
            reference.contains(&format!("`{}`", descriptor.name)),
            "tools reference omits built-in action {}",
            descriptor.name
        );
    }
}
