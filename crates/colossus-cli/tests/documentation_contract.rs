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

const LEGACY_OPERATOR_SIGNATURES: &[&str] = &[
    "uv run colossus",
    "colossus-rs",
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

#[test]
fn static_documentation_site_is_complete_searchable_and_python_free() {
    let configuration = read("book.toml");
    for required in [
        "src = \"docs\"",
        "build-dir = \"site\"",
        "create-missing = false",
        "site-url = \"/Colossus/\"",
        "edit-url-template = \"https://github.com/obscuritylabs/Colossus/edit/main/{path}\"",
        "[output.html.search]",
        "enable = true",
    ] {
        assert!(
            configuration.contains(required),
            "book.toml is missing {required:?}"
        );
    }

    let summary = read("docs/SUMMARY.md");
    for entry in fs::read_dir(repository_root().join("docs")).expect("read docs directory") {
        let path = entry.expect("documentation entry").path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if path.extension().and_then(|value| value.to_str()) == Some("md") && name != "SUMMARY.md" {
            assert!(
                summary.contains(&format!("]({name})")),
                "docs/{name} is absent from the published navigation"
            );
        }
    }

    let workflow = read(".github/workflows/docs.yml");
    for required in [
        "MDBOOK_VERSION: \"0.5.4\"",
        "cargo install mdbook --version",
        "mdbook\" build",
        "actions/configure-pages@v5",
        "actions/upload-pages-artifact@v4",
        "actions/deploy-pages@v4",
    ] {
        assert!(
            workflow.contains(required),
            "documentation workflow is missing {required:?}"
        );
    }
    for forbidden in ["setup-python", "pip install", "uv run", "pyproject.toml"] {
        assert!(
            !workflow.contains(forbidden),
            "documentation workflow reintroduced Python tooling via {forbidden:?}"
        );
    }
}

#[test]
fn static_documentation_links_resolve_inside_the_published_source_tree() {
    let docs = repository_root().join("docs");
    for entry in fs::read_dir(&docs).expect("read docs directory") {
        let path = entry.expect("documentation entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let document = fs::read_to_string(&path).expect("read documentation page");
        let mut remainder = document.as_str();
        while let Some((_, after_open)) = remainder.split_once("](") {
            let Some((target, after_close)) = after_open.split_once(')') else {
                panic!("{} contains an unterminated Markdown link", path.display());
            };
            remainder = after_close;
            if target.starts_with("https://")
                || target.starts_with("http://")
                || target.starts_with("mailto:")
                || target.starts_with('#')
            {
                continue;
            }
            let relative = target.split('#').next().expect("relative link target");
            assert!(
                docs.join(relative).is_file(),
                "{} links to missing published source {target:?}",
                path.display()
            );
        }
    }
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
        for signature in LEGACY_OPERATOR_SIGNATURES {
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
    let binary = env!("CARGO_BIN_EXE_colossus");
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
