//! Rust-first operator documentation acceptance.

use colossus_runtime::RuntimeConfig;
use colossus_workflow::validate_definition;
use std::{fs, path::Path, process::Command};

const OPERATOR_DOCS: &[&str] = &[
    "README.md",
    "docs/README.md",
    "docs/GETTING_STARTED.md",
    "docs/INSTALLATION.md",
    "docs/CONFIGURATION.md",
    "docs/USER_GUIDE.md",
    "docs/TOOLS.md",
    "docs/CONTEXT.md",
    "docs/SKILLS.md",
    "docs/PACKS.md",
    "docs/INTEGRATIONS.md",
    "docs/WORKFLOWS.md",
    "docs/TROUBLESHOOTING.md",
    "docs/RELEASE.md",
    "docs/OFFLINE_AIRGAP.md",
    "docs/BUNDLE_FORMAT.md",
];

const PYTHON_OPERATOR_SIGNATURES: &[&str] = &[
    "uv run colossus",
    "`config.json`",
    "local_openai_chat",
    "sqlite_fts",
    "`--workspace`",
    "--credential-ref ",
    "Typer CLI",
    "prompt-toolkit",
    "wheelhouse/colossus",
    "/status",
    "/workspace",
    "/compact",
    "/session latest",
    "/context snapshots",
    "/skill drop",
    "/research on",
    "/agents resume",
];

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

#[test]
fn active_operator_docs_do_not_reintroduce_python_runtime_commands() {
    for path in OPERATOR_DOCS {
        let document = read(path);
        for signature in PYTHON_OPERATOR_SIGNATURES {
            assert!(
                !document.contains(signature),
                "{path} reintroduced legacy operator signature {signature:?}"
            );
        }
    }
}

#[test]
fn published_config_and_workflow_examples_are_accepted_by_the_rust_parsers() {
    let configuration = read("docs/CONFIGURATION.md");
    RuntimeConfig::from_yaml(marked_yaml(&configuration, "rust-config-example"))
        .expect("documented Rust configuration must parse");

    let workflows = read("docs/WORKFLOWS.md");
    validate_definition(marked_yaml(&workflows, "rust-workflow-example"))
        .expect("documented workflow must validate");
}

#[test]
fn documented_command_families_are_real_clap_routes() {
    let binary = env!("CARGO_BIN_EXE_colossus-rs");
    let routes: &[&[&str]] = &[
        &["config", "init"],
        &["audit", "anchor-status"],
        &["policy", "doctor"],
        &["state", "doctor"],
        &["sandbox", "doctor"],
        &["provider", "models"],
        &["models", "route"],
        &["sessions", "messages"],
        &["context", "restore"],
        &["tasks", "create"],
        &["decisions", "create"],
        &["plans", "approve"],
        &["goals", "run"],
        &["agents", "queue"],
        &["memories", "index", "rebuild"],
        &["research", "run"],
        &["skills", "scaffold"],
        &["packs", "trust", "add"],
        &["bundle", "key-info"],
        &["bundle", "build"],
        &["bundle", "install"],
        &["workflow", "input"],
        &["integrations", "import-openapi"],
        &["mcp", "call"],
        &["worker"],
    ];
    for route in routes {
        let output = Command::new(binary)
            .args(*route)
            .arg("--help")
            .output()
            .unwrap_or_else(|error| panic!("run {route:?}: {error}"));
        assert!(
            output.status.success(),
            "documented route {route:?} is invalid: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
