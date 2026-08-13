//! Audience-first documentation acceptance.

#[path = "support/process.rs"]
mod process_support;

use colossus_access::builtin_action_descriptors;
use colossus_runtime::RuntimeConfig;
use colossus_tools::builtin_specs;
use colossus_workflow::validate_definition;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::tempdir;

const LEGACY_PUBLIC_SIGNATURES: &[&str] = &[
    "uv run colossus",
    "colossus-rs",
    "`config.json`",
    "local_openai_chat",
    "sqlite_fts",
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
    "/agents resume",
];

const HISTORICAL_HTML_ROUTES: &[&str] = &[
    "ACCESS_PROFILES.html",
    "ARCHITECTURE.html",
    "BUNDLE_FORMAT.html",
    "CONFIGURATION.html",
    "CONTEXT.html",
    "CONTRIBUTING.html",
    "CRATE_STRUCTURE.html",
    "FEATURE_INVENTORY.html",
    "GETTING_STARTED.html",
    "INSTALLATION.html",
    "INTEGRATIONS.html",
    "OFFLINE_AIRGAP.html",
    "PACKS.html",
    "README.html",
    "RELEASE.html",
    "RUST_ACCEPTANCE_MATRIX.html",
    "RUST_RECONSTRUCTION.html",
    "SEARCH.html",
    "SECURITY.html",
    "SKILLS.html",
    "TERMINAL_UX.html",
    "TOOLS.html",
    "TROUBLESHOOTING.html",
    "USER_GUIDE.html",
    "WORKFLOWS.html",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Audience {
    User,
    Operator,
    Developer,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
enum PageType {
    #[serde(rename = "tutorial")]
    Tutorial,
    #[serde(rename = "how-to")]
    HowTo,
    #[serde(rename = "concept")]
    Concept,
    #[serde(rename = "reference")]
    Reference,
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    title: String,
    description: String,
    audience: Audience,
    #[serde(rename = "type")]
    page_type: PageType,
}

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

fn markdown_pages(root: &Path) -> Vec<PathBuf> {
    fn visit(directory: &Path, pages: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        {
            let entry = entry.expect("documentation directory entry");
            let path = entry.path();
            let file_type = entry.file_type().expect("documentation entry type");
            if file_type.is_dir() {
                visit(&path, pages);
            } else if file_type.is_file()
                && path.extension().and_then(|value| value.to_str()) == Some("md")
            {
                pages.push(path);
            }
        }
    }

    let mut pages = Vec::new();
    visit(root, &mut pages);
    pages.sort();
    pages
}

fn parse_frontmatter(path: &Path, document: &str) -> Frontmatter {
    let after_open = document
        .strip_prefix("---\n")
        .unwrap_or_else(|| panic!("{} has no YAML frontmatter", path.display()));
    let (yaml, _) = after_open
        .split_once("\n---\n")
        .unwrap_or_else(|| panic!("{} has unterminated YAML frontmatter", path.display()));
    let metadata: Frontmatter = serde_saphyr::from_str(yaml)
        .unwrap_or_else(|error| panic!("parse {} frontmatter: {error}", path.display()));
    assert!(
        !metadata.title.trim().is_empty(),
        "{} has an empty title",
        path.display()
    );
    assert!(
        !metadata.description.trim().is_empty(),
        "{} has an empty description",
        path.display()
    );
    metadata
}

#[test]
fn public_installation_docs_keep_the_complete_distribution_path() {
    let readme = read("README.md");
    for required in [
        "releases/latest/download/colossus-install.sh | sh",
        "releases/latest/download/colossus-install.ps1 | iex",
        "[installation guide](docs/get-started/install.md)",
        "colossus update check",
        "colossus update",
    ] {
        assert!(
            readme.contains(required),
            "README is missing public installation contract {required:?}"
        );
    }
    assert!(
        !readme.contains("Download the archive for your platform"),
        "README must keep the direct installer as the primary installation path"
    );

    let install = read("docs/get-started/install.md");
    for required in [
        "curl -fSLo colossus-install.sh",
        "--version vX.Y.Z",
        "-Version vX.Y.Z",
        "nix profile install github:obscuritylabs/Colossus",
        "obscuritylabs/homebrew-tap",
        "brew install obscuritylabs/tap/colossus",
        "brew upgrade obscuritylabs/tap/colossus",
        "sha256sum --check",
        "Get-FileHash $archive -Algorithm SHA256",
        "colossus update check",
        "colossus update --version vX.Y.Z",
        "Installation receipt",
        "Uninstall a direct installation",
    ] {
        assert!(
            install.contains(required),
            "installation guide is missing public distribution contract {required:?}"
        );
    }
    assert!(
        !install.contains("mirrored to the planned"),
        "installation guide must describe the published Homebrew tap"
    );
}

#[test]
fn zensical_site_is_pinned_searchable_and_complete() {
    assert!(
        !repository_root().join("book.toml").exists(),
        "book.toml must be removed after the atomic Zensical cutover"
    );
    assert!(
        !repository_root().join("docs/SUMMARY.md").exists(),
        "mdBook navigation must not remain in the published tree"
    );

    let configuration = read("zensical.toml");
    for required in [
        "[project]",
        "site_name = \"Colossus\"",
        "site_url = \"https://obscuritylabs.github.io/Colossus/\"",
        "docs_dir = \"docs\"",
        "site_dir = \"site\"",
        "use_directory_urls = true",
        "repo_url = \"https://github.com/obscuritylabs/Colossus\"",
        "edit_uri = \"https://github.com/obscuritylabs/Colossus/edit/main/docs/\"",
        "variant = \"modern\"",
        "extra_css = [\"stylesheets/extra.css\"]",
        "extra_javascript = [\"assets/vendor/mermaid-11.15.0.min.js\"]",
        "pymdownx.superfences",
        "name = \"mermaid\"",
        "navigation.tabs",
        "navigation.tabs.sticky",
        "navigation.indexes",
        "navigation.path",
        "navigation.top",
        "content.code.copy",
    ] {
        assert!(
            configuration.contains(required),
            "zensical.toml is missing {required:?}"
        );
    }
    assert!(
        !configuration.contains("internal/documentation"),
        "repository-only documentation entered the public configuration"
    );
    assert!(
        !configuration.contains("\"navigation.sections\""),
        "nested navigation must remain collapsible on desktop"
    );
    for (label, page) in [
        ("Research overview", "docs/use/research-search.md"),
        ("Deep research", "docs/use/deep-research.md"),
        ("Web search", "docs/use/web-search.md"),
    ] {
        let nav_page = page
            .strip_prefix("docs/")
            .expect("public documentation page");
        assert!(
            configuration.contains(&format!("{{ \"{label}\" = \"{nav_page}\" }}")),
            "Zensical navigation is missing the {label} page"
        );
        assert!(
            repository_root().join(page).is_file(),
            "{label} source page is missing"
        );
    }
    let research_overview = read("docs/use/research-search.md");
    for legacy_anchor in [
        "goal",
        "prerequisites",
        "steps",
        "1-run-repository-only-research",
        "2-inspect-evidence-and-claims",
        "3-diagnose-a-web-route-directly",
        "4-run-research-with-selected-lanes",
        "search-routing",
        "expected-result",
        "verification",
        "failure-path",
        "next-step",
    ] {
        assert!(
            research_overview.contains(&format!("id=\"{legacy_anchor}\"")),
            "research overview is missing legacy anchor #{legacy_anchor}"
        );
    }
    let mermaid_runtime = repository_root().join("docs/assets/vendor/mermaid-11.15.0.min.js");
    assert!(
        mermaid_runtime.is_file(),
        "the pinned repository-local Mermaid runtime is missing"
    );
    assert!(
        fs::metadata(&mermaid_runtime)
            .expect("read local Mermaid runtime metadata")
            .len()
            > 1_000_000,
        "the local Mermaid runtime is not the reviewed standalone distribution"
    );
    assert!(
        repository_root()
            .join("docs/assets/vendor/mermaid-LICENSE.txt")
            .is_file(),
        "the vendored Mermaid license is missing"
    );
    let docs = repository_root().join("docs");
    let pages = markdown_pages(&docs);
    assert!(
        pages.len() >= 50,
        "expected the complete nested documentation IA"
    );
    let mut mermaid_diagrams = 0;
    for path in &pages {
        let document = fs::read_to_string(path).expect("read public documentation");
        let metadata = parse_frontmatter(path, &document);
        let relative = path.strip_prefix(&docs).expect("page under docs");
        if relative != Path::new("404.md") {
            let relative = relative.to_string_lossy().replace('\\', "/");
            assert!(
                configuration.contains(&format!("\"{relative}\"")),
                "{} is absent from explicit Zensical navigation",
                path.display()
            );
        }
        if matches!(metadata.page_type, PageType::Tutorial | PageType::HowTo) {
            for heading in [
                "## Goal",
                "## Prerequisites",
                "## Steps",
                "## Expected result",
                "## Verification",
                "## Failure path",
                "## Next step",
            ] {
                assert!(
                    document.contains(heading),
                    "{} is a {:?} page without required section {heading:?}",
                    path.display(),
                    metadata.page_type
                );
            }
        }
        let page_diagrams = document.matches("```mermaid").count();
        if page_diagrams > 0 {
            mermaid_diagrams += page_diagrams;
            assert!(
                document.matches("class=\"diagram-scroll").count() >= page_diagrams,
                "{} has a Mermaid diagram without a narrow-screen scroll region",
                path.display()
            );
            assert!(
                document.matches("role=\"region\"").count() >= page_diagrams
                    && document.matches("tabindex=\"0\"").count() >= page_diagrams
                    && document.matches("aria-label=\"").count() >= page_diagrams,
                "{} has a Mermaid diagram without a labeled keyboard-focusable region",
                path.display()
            );
        }
    }
    let access_diagram_source = docs.join("diagrams/access-resolution.drawio");
    let access_diagram_export = docs.join("diagrams/access-resolution.svg");
    assert!(
        access_diagram_source.is_file() && access_diagram_export.is_file(),
        "the editable and exported access-resolution diagrams must ship together"
    );
    let access_page = read("docs/admin/access-and-approvals.md");
    assert!(
        access_page.contains("../diagrams/access-resolution.svg")
            && access_page.contains("../diagrams/access-resolution.drawio"),
        "the access guide must embed the SVG and link the editable Draw.io source"
    );
    assert_eq!(
        mermaid_diagrams + 1,
        8,
        "the maintained product and architecture diagram set changed unexpectedly"
    );
    assert!(
        read("docs/develop/ci-cd.md").contains("Tiered CI and release flow diagram"),
        "the CI/CD guide must include its accessible tier-flow diagram"
    );
}

#[test]
fn pinned_container_wrapper_and_pages_workflow_share_one_build_interface() {
    let wrapper = read("scripts/docs-site");
    let reviewed_image = concat!(
        "zensical/zensical:0.0.50@",
        "sha256:a67f689607908b47b9979ff8213906477f78826e9f8565f012e06071f883e973"
    );
    assert!(
        wrapper.contains(reviewed_image),
        "scripts/docs-site does not pin the reviewed image {reviewed_image}"
    );
    for required in ["build", "--clean", "--strict", "serve", "docker"] {
        assert!(
            wrapper.contains(required),
            "scripts/docs-site is missing {required:?}"
        );
    }

    let workflow = read(".github/workflows/docs.yml");
    for required in [
        "./scripts/docs-site build",
        "actions/configure-pages@45bfe0192ca1faeb007ade9deae92b16b8254a0d # v6.0.0",
        "actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9 # v5.0.0",
        "actions/deploy-pages@cd2ce8fcbc39b97be8ca5fce6e763baed58fa128 # v5.0.0",
        "pages: write",
        "branches: [\"main\"]",
    ] {
        assert!(
            workflow.contains(required),
            "documentation workflow is missing {required:?}"
        );
    }
    assert!(
        !workflow.contains("pull_request:"),
        "documentation deployment must not duplicate PR validation"
    );
    let pr = read(".github/workflows/pr.yml");
    for required in [
        "Documentation PR build",
        "./scripts/docs-site build",
        "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1",
    ] {
        assert!(
            pr.contains(required),
            "PR documentation is missing {required:?}"
        );
    }
    for forbidden in [
        "setup-python",
        "pip install",
        "uv run",
        "pyproject.toml",
        "mdbook",
        "actions/cache",
    ] {
        assert!(
            !workflow.contains(forbidden),
            "documentation workflow bypasses the isolated build contract via {forbidden:?}"
        );
    }
}

#[test]
fn internal_archives_and_legacy_routes_are_explicitly_accounted_for() {
    let docs = repository_root().join("docs");
    for retired in [
        "FEATURE_INVENTORY.md",
        "RELEASE.md",
        "RUST_ACCEPTANCE_MATRIX.md",
        "RUST_RECONSTRUCTION.md",
        "TERMINAL_UX.md",
    ] {
        assert!(
            !docs.join(retired).exists(),
            "{retired} must not remain in the published root"
        );
    }

    let archive = repository_root().join("internal/documentation");
    let archived_pages = markdown_pages(&archive);
    assert!(
        archived_pages.len() >= 5,
        "internal documentation must retain history and acceptance evidence"
    );
    for path in archived_pages {
        let document = fs::read_to_string(&path).expect("read internal documentation");
        assert!(
            document.contains("status:"),
            "{} does not declare current or archived status",
            path.display()
        );
        assert!(
            document.contains("replacement"),
            "{} does not identify replacement public documentation",
            path.display()
        );
    }

    let redirects = read("documentation/legacy-routes.tsv");
    for route in HISTORICAL_HTML_ROUTES {
        assert!(
            redirects.contains(route),
            "legacy route manifest does not cover {route}"
        );
    }
    for required in [
        "FEATURE_INVENTORY.html",
        "22-delivery-status=",
        "SEARCH.html",
        "search-contract=4-read-the-normalized-response",
        "fragment",
    ] {
        assert!(
            redirects.contains(required),
            "legacy route manifest is missing {required:?} compatibility metadata"
        );
    }

    let generator = read("scripts/generate-doc-redirects");
    for required in [
        "rel=\"canonical\"",
        "noindex, nofollow",
        "window.location.hash",
        "http-equiv=\"refresh\"",
        "<main><p>This documentation moved to <a",
    ] {
        assert!(
            generator.contains(required),
            "legacy redirect generator is missing {required:?}"
        );
    }

    let capture = read("documentation/tui-offline-session.txt");
    assert_eq!(
        capture.lines().count(),
        12,
        "the real TUI capture must remain a fixed 112x12 frame"
    );
    let capture_width = capture
        .lines()
        .map(|line| line.chars().count())
        .max()
        .expect("the real TUI capture must not be empty");
    assert_eq!(
        capture_width, 112,
        "the real TUI capture must retain its 112-column display width"
    );
    assert!(
        capture.lines().all(|line| line.chars().count() <= 112),
        "the real TUI capture must not overflow its 112-column display width"
    );
    for marker in [
        "› You",
        "● Colossus",
        "primary:echo@echo",
        "approval=ask",
        "status=ok",
    ] {
        assert!(
            capture.contains(marker),
            "the real TUI capture is missing {marker:?}"
        );
    }
    let screenshot =
        fs::read(repository_root().join("docs/assets/screenshots/tui-offline-session.png"))
            .expect("read real TUI screenshot");
    assert!(
        screenshot.starts_with(b"\x89PNG\r\n\x1a\n"),
        "the homepage TUI screenshot must be a literal PNG capture"
    );
    assert!(
        screenshot.len() >= 33,
        "the homepage TUI screenshot must contain a complete PNG IHDR chunk"
    );
    assert_eq!(
        u32::from_be_bytes(screenshot[8..12].try_into().expect("IHDR length")),
        13,
        "the homepage TUI screenshot must begin with a 13-byte IHDR chunk"
    );
    assert_eq!(
        &screenshot[12..16],
        b"IHDR",
        "the homepage TUI screenshot must begin with an IHDR chunk"
    );
    let width = u32::from_be_bytes(screenshot[16..20].try_into().expect("PNG width"));
    let height = u32::from_be_bytes(screenshot[20..24].try_into().expect("PNG height"));
    assert_eq!(
        (width, height),
        (1184, 304),
        "the homepage TUI screenshot must retain its reproducible compact composition"
    );
    let renderer = read("scripts/render-docs-tui-capture");
    for marker in [
        "pango-view",
        "Menlo 17",
        "#06182a",
        "#dceeff",
        "--margin=\"28 32\"",
        "--antialias=gray",
    ] {
        assert!(
            renderer.contains(marker),
            "the TUI screenshot renderer is missing {marker:?}"
        );
    }
    assert!(
        !repository_root()
            .join("docs/assets/screenshots/tui-offline-session.svg")
            .exists(),
        "the former illustrative TUI vector must not return"
    );
}

#[test]
fn public_links_resolve_from_nested_source_pages() {
    let docs = repository_root().join("docs");
    let canonical_docs = docs.canonicalize().expect("canonical docs directory");
    for path in markdown_pages(&docs) {
        let document = fs::read_to_string(&path).expect("read documentation page");
        let mut remainder = document.as_str();
        while let Some((_, after_open)) = remainder.split_once("](") {
            let Some((raw_target, after_close)) = after_open.split_once(')') else {
                panic!("{} contains an unterminated Markdown link", path.display());
            };
            remainder = after_close;
            let target = raw_target.trim().trim_matches(['<', '>']);
            if target.starts_with("https://")
                || target.starts_with("http://")
                || target.starts_with("mailto:")
                || target.starts_with('#')
            {
                continue;
            }
            let relative = target.split('#').next().expect("relative link target");
            if relative.is_empty() {
                continue;
            }
            assert!(
                !relative.starts_with('/'),
                "{} uses non-portable site-absolute link {target:?}",
                path.display()
            );
            let candidate = path.parent().expect("page parent").join(relative);
            let candidate = if candidate.is_dir() {
                candidate.join("index.md")
            } else {
                candidate
            };
            assert!(
                candidate.is_file(),
                "{} links to missing published source {target:?}",
                path.display()
            );
            let canonical_candidate = candidate
                .canonicalize()
                .unwrap_or_else(|error| panic!("canonicalize {}: {error}", candidate.display()));
            assert!(
                canonical_candidate.starts_with(&canonical_docs),
                "{} links outside the published source tree via {target:?}",
                path.display()
            );
        }
    }
}

#[test]
fn user_and_operator_pages_use_only_current_installed_binary_commands() {
    let docs = repository_root().join("docs");
    for path in markdown_pages(&docs) {
        let document = fs::read_to_string(&path).expect("read public documentation");
        let metadata = parse_frontmatter(&path, &document);
        if metadata.audience == Audience::Developer {
            continue;
        }
        for signature in LEGACY_PUBLIC_SIGNATURES {
            assert!(
                !document.contains(signature),
                "{} reintroduced legacy public signature {signature:?}",
                path.display()
            );
        }
        assert!(
            !document.contains("cargo "),
            "{} puts source-build commands in a user or operator page",
            path.display()
        );

        if !path.ends_with("get-started/upgrade-compatibility.md") {
            for historical in ["Python 0.5", "python-v0.5.0", "python-legacy"] {
                assert!(
                    !document.contains(historical),
                    "{} duplicates cutover history outside the compatibility page",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn colossus_home_configuration_and_instruction_contract_is_published() {
    let home = read("docs/reference/colossus-home.md");
    for required in [
        "COLOSSUS_HOME",
        "workspaces/<partition-id>/cli/",
        "workspaces/<partition-id>/desktop/",
        "Explicit `--config PATH`",
        "`<workspace>/.colossus/config.yaml`",
        "complete replacements",
        "`configScope`",
        "config init --local",
        "`storage.location`",
        "`home_workspace`",
        "64 KiB",
        "128 KiB",
        "Goal iterations",
        "`instruction_sources`",
        "snapshot_refresh: top_level_run",
        "cannot add tools",
        "preserves `$COLOSSUS_HOME`",
    ] {
        assert!(
            home.contains(required),
            "Colossus home reference is missing {required:?}"
        );
    }

    let navigation = read("zensical.toml");
    assert!(navigation.contains("reference/colossus-home.md"));
    for page in [
        "README.md",
        "docs/get-started/install.md",
        "docs/get-started/quickstart.md",
        "docs/get-started/desktop.md",
        "docs/reference/configuration.md",
        "docs/reference/cli.md",
    ] {
        assert!(
            read(page).contains("colossus-home.md"),
            "{page} must link the home-resolution authority"
        );
    }
}

#[test]
fn home_upgrade_examples_and_desktop_terminal_boundaries_stay_consistent() {
    for path in markdown_pages(&repository_root().join("docs")) {
        let document = fs::read_to_string(&path).expect("read documentation page");
        assert!(
            !document.contains(".colosus"),
            "{} misspells the .colossus directory",
            path.display()
        );
    }

    let offline = read("docs/admin/offline-airgap.md");
    assert!(offline.contains("colossus -w . config init"));
    assert!(
        !offline.contains("--config .colossus/config.yaml"),
        "offline commands must not select a local config that global init did not create"
    );

    let desktop = read("docs/get-started/desktop.md");
    let terminal = read("docs/use/terminal-ui.md");
    for document in [&desktop, &terminal] {
        assert!(document.contains("/bin/zsh -l"));
        assert!(document.contains("outside Colossus policy"));
        assert!(!document.contains("rejects arbitrary Shell PTYs"));
    }

    let storage = read("docs/reference/configuration/storage.md");
    assert!(storage.contains("with `home_workspace`, a confined relative path"));
    let administration = read("docs/admin/configuration.md");
    assert!(administration.contains("anchor_path: secure-anchor.json"));

    let routes = read("docs/reference/cli.md");
    for flag in ["--development", "--from PATH", "--storage-keys MODE"] {
        assert!(routes.contains(flag), "CLI route index is missing {flag}");
    }
}

#[test]
fn sparse_full_access_defaults_and_warnings_are_published_consistently() {
    let quickstart = read("docs/get-started/quickstart.md");
    for required in [
        "access.profile: allow_all",
        "sandbox.backend: danger_full_access",
        "danger-full-access warning also appears on stderr",
        "JSON stdout stays clean",
    ] {
        assert!(
            quickstart.contains(required),
            "quickstart is missing sparse/full-access contract {required:?}"
        );
    }

    let upgrade = read("docs/get-started/upgrade-compatibility.md");
    assert!(upgrade.contains("--sandbox-profile workspace-development"));
    assert!(upgrade.contains("applies to existing sparse files without a schema-version bump"));

    let cli = read("docs/reference/cli.md");
    for required in [
        "config init --development --from PATH",
        "preserves the explicitly supplied",
        "danger-full-access posture warning",
        "emitted on stderr even when",
        "contaminates the one-value JSON stdout contract",
    ] {
        assert!(
            cli.contains(required),
            "CLI reference is missing {required:?}"
        );
    }

    let desktop = read("docs/get-started/desktop.md");
    for required in [
        "Full access",
        "default access profile",
        "boundary for fresh Managed Local settings",
        "Schema-v1–v3 migrations preserve",
        "persistent warning",
        "Workspace isolated",
        "Offline isolated",
        "not an air gap",
        "`network.http`, `web.fetch`,",
        "authentication/refresh destinations",
        "do not make the generic",
    ] {
        assert!(
            desktop.contains(required),
            "Desktop guide is missing execution-boundary contract {required:?}"
        );
    }

    let opa = read("docs/admin/policy-opa.md");
    for required in [
        "resource_authority: ambient",
        "`resource_authority` defaults to",
        "`declared` when omitted",
        "does not silently rewrite an OPA response",
    ] {
        assert!(opa.contains(required), "OPA guide is missing {required:?}");
    }

    let sandbox = read("docs/reference/configuration/sandbox.md");
    for required in [
        "canonical plaintext HTTP outside loopback",
        "no TLS",
        "server authentication",
        "Adding the list does not narrow ambient",
    ] {
        assert!(
            sandbox.contains(required),
            "sandbox reference is missing ambient-authority contract {required:?}"
        );
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
fn published_config_and_workflow_examples_are_accepted_by_rust_parsers() {
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
        "the complete documented baseline must stay identical to `config show` for a minimal global configuration"
    );

    let workflows = read("docs/extend/workflows/authoring.md");
    validate_definition(marked_yaml(&workflows, "rust-workflow-example"))
        .expect("documented workflow must validate");

    let workflow_reference = read("docs/reference/workflow-schema.md");
    validate_definition(marked_yaml(&workflow_reference, "rust-workflow-example"))
        .expect("reference workflow must validate");
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
fn provider_task_guides_are_linked_complete_and_parser_backed() {
    let chooser = read("docs/use/providers/index.md");
    let use_overview = read("docs/use/index.md");
    let navigation = read("zensical.toml");
    let guides = [
        (
            "Codex or ChatGPT subscription",
            "docs/use/providers/codex-chatgpt.md",
            "kind: open_ai_codex",
            "credentialReference: codex:default",
            "https://chatgpt.com",
        ),
        (
            "OpenAI API",
            "docs/use/providers/openai-api.md",
            "kind: open_ai_responses",
            "credentialReference: env:OPENAI_API_KEY",
            "https://api.openai.com",
        ),
        (
            "OpenRouter",
            "docs/use/providers/openrouter.md",
            "kind: open_ai_compatible",
            "credentialReference: env:OPENROUTER_API_KEY",
            "https://openrouter.ai",
        ),
        (
            "Local models",
            "docs/use/providers/local-models.md",
            "kind: open_ai_compatible",
            "credentialReference: null",
            "http://127.0.0.1:11434",
        ),
        (
            "Other OpenAI-compatible endpoints",
            "docs/use/providers/openai-compatible.md",
            "kind: open_ai_compatible",
            "credentialReference: env:COLOSSUS_MODEL_TOKEN",
            "https://models.example.com",
        ),
    ];
    let baseline = read("docs/reference/configuration.md");
    let baseline = marked_yaml(&baseline, "rust-config-example");

    for (label, path, kind, credential, origin) in guides {
        let relative_path = path.strip_prefix("docs/").expect("public guide path");
        let document = read(path);
        for required in [
            "schemaVersion: 2",
            kind,
            credential,
            origin,
            "models route primary",
            "provider doctor",
            "models doctor",
            "Reply with exactly: connected",
            "../../reference/configuration/providers-models.md",
            "../../admin/providers-routing.md",
        ] {
            assert!(
                document.contains(required),
                "{path} is missing provider-guide contract {required:?}"
            );
        }
        let chooser_target = relative_path.trim_start_matches("use/providers/");
        assert!(
            chooser.contains(&format!("]({chooser_target})")),
            "provider chooser does not link {label}"
        );
        let overview_target = relative_path.trim_start_matches("use/");
        assert!(
            use_overview.contains(overview_target),
            "Use overview does not link {label}"
        );
        assert!(
            navigation.contains(relative_path),
            "Zensical navigation does not include {label}"
        );

        let mut configuration: serde_json::Value =
            serde_saphyr::from_str(baseline).expect("baseline configuration YAML");
        let overlay: serde_json::Value =
            serde_saphyr::from_str(marked_yaml(&document, "provider-guide-config"))
                .unwrap_or_else(|error| panic!("parse {path} provider overlay: {error}"));
        let overlay = overlay
            .as_object()
            .expect("provider overlay mapping")
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
            .unwrap_or_else(|error| panic!("{path} provider overlay must parse: {error}"));
    }
}

#[test]
fn repository_workflow_examples_are_accepted_by_the_rust_parser() {
    let directory = repository_root().join("examples/workflows");
    let mut paths = fs::read_dir(&directory)
        .expect("read workflow examples")
        .map(|entry| entry.expect("workflow example entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("yaml"))
        .collect::<Vec<_>>();
    paths.sort();
    assert!(
        paths.len() >= 7,
        "advanced workflow example suite is unexpectedly small"
    );

    let mut identities = BTreeSet::new();
    for path in paths {
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
fn repository_agent_ask_examples_are_bounded_documented_and_portable() {
    let directory = repository_root().join("examples/asks");
    let readme = fs::read_to_string(directory.join("README.md")).expect("read ask README");
    let mut paths = fs::read_dir(&directory)
        .expect("read ask examples")
        .map(|entry| entry.expect("ask example entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("txt"))
        .collect::<Vec<_>>();
    paths.sort();
    assert!(
        paths.len() >= 10,
        "agent ask example suite is unexpectedly small"
    );

    let mut names = BTreeSet::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("portable UTF-8 ask filename");
        assert!(
            names.insert(name.to_owned()),
            "duplicate ask example {name}"
        );
        assert!(
            readme.contains(&format!("`{name}`")),
            "ask README omits {name}"
        );

        let prompt = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            !prompt.trim().is_empty() && prompt.len() <= 4_096,
            "{} must contain one bounded prompt",
            path.display()
        );
        assert!(
            prompt.ends_with('\n'),
            "{} must end with a newline",
            path.display()
        );
        assert!(
            !prompt
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t')),
            "{} contains unsafe control characters",
            path.display()
        );
        for absolute_prefix in ["/Users/", "/home/", "C:\\Users\\"] {
            assert!(
                !prompt.contains(absolute_prefix),
                "{} contains non-portable absolute path prefix {absolute_prefix:?}",
                path.display()
            );
        }
    }

    for fixture_path in ["Cargo.toml", "README.md", "src/lib.rs"] {
        assert!(
            directory.join("fixture").join(fixture_path).is_file(),
            "ask implementation fixture omits {fixture_path}"
        );
    }
}

#[test]
fn repository_sdk_examples_are_cross_language_bounded_and_documented() {
    let directory = repository_root().join("examples/sdk");
    let readme = fs::read_to_string(directory.join("README.md")).expect("read SDK example README");
    for required in [
        "crates/colossus-sdk/examples/durable_run.rs",
        "sdk/python/examples/durable_run.py",
        "sdk/typescript/examples/durable-run.ts",
        "sdk/go/examples/durable-run/durable_run.go",
        "cargo run -p colossus-cli --example sdk_ephemeral_local",
        "sdk/python/examples/live_run.py",
        "sdk/typescript/examples/live-run.ts",
        "sdk/go/examples/live-run/main.go",
        "anonymous child-stdin pipe",
        "never enrolls an application",
        "serializes the bearer into argv",
        "never automatically retry",
        "OS credential-store",
        "openapi.sdk-demo.getstatus",
    ] {
        assert!(
            readme.contains(required),
            "SDK example README omits {required:?}"
        );
    }

    for source in [
        "crates/colossus-cli/examples/sdk_ephemeral_local.rs",
        "crates/colossus-sdk/examples/durable_run.rs",
        "sdk/python/examples/durable_run.py",
        "sdk/python/examples/live_run.py",
        "sdk/typescript/examples/durable-run.ts",
        "sdk/typescript/examples/live-run.ts",
        "sdk/go/examples/durable-run/durable_run.go",
        "sdk/go/examples/live-run/main.go",
    ] {
        assert!(
            repository_root().join(source).is_file(),
            "SDK example source is missing: {source}"
        );
    }

    let scenario_directory = directory.join("scenarios");
    let mut scenarios = fs::read_dir(&scenario_directory)
        .expect("read SDK scenarios")
        .map(|entry| entry.expect("SDK scenario entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("txt"))
        .collect::<Vec<_>>();
    scenarios.sort();
    assert_eq!(
        scenarios.len(),
        6,
        "SDK scenario suite must remain explicit"
    );
    for path in scenarios {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("portable UTF-8 SDK scenario filename");
        assert!(
            readme.contains(&format!("`{name}`")),
            "SDK README omits scenario {name}"
        );
        let prompt = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            !prompt.trim().is_empty() && prompt.len() <= 2_048 && prompt.ends_with('\n'),
            "{} must contain one newline-terminated bounded prompt",
            path.display()
        );
        for absolute_prefix in ["/Users/", "/home/", "C:\\Users\\"] {
            assert!(
                !prompt.contains(absolute_prefix),
                "{} contains non-portable absolute path prefix {absolute_prefix:?}",
                path.display()
            );
        }
    }

    let openapi =
        fs::read(directory.join("integration/openapi.json")).expect("read SDK OpenAPI fixture");
    let openapi: serde_json::Value =
        serde_json::from_slice(&openapi).expect("SDK OpenAPI fixture must be valid JSON");
    assert_eq!(openapi["openapi"], "3.1.0");
    assert_eq!(
        openapi["paths"]["/status/{service}"]["get"]["operationId"],
        "getStatus"
    );
    assert!(
        directory.join("integration/server.py").is_file(),
        "SDK integration server fixture is missing"
    );
    assert!(
        directory.join("provider-failure/server.py").is_file(),
        "SDK provider failure fixture is missing"
    );
}

#[test]
fn tools_and_action_reference_covers_the_executable_catalog() {
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

#[test]
fn documented_command_families_are_real_clap_routes() {
    let binary = env!("CARGO_BIN_EXE_colossus");
    let isolated_home = tempdir().expect("isolated command home");
    let routes: &[&[&str]] = &[
        &["config", "init"],
        &["audit", "anchor-status"],
        &["policy", "doctor"],
        &["state", "doctor"],
        &["sandbox", "doctor"],
        &["codex", "login"],
        &["codex", "status"],
        &["codex", "logout"],
        &["provider", "doctor"],
        &["provider", "models"],
        &["search", "profiles"],
        &["search", "query"],
        &["models", "route"],
        &["models", "doctor"],
        &["run"],
        &["artifacts", "upload"],
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
        &["workflow", "webhook", "create"],
        &["workflow", "webhook", "serve"],
        &["integrations", "import-openapi"],
        &["mcp", "call"],
        &["worker"],
    ];
    for route in routes {
        let mut command = Command::new(binary);
        process_support::isolate_user_home(&mut command, isolated_home.path());
        let output = command
            .args(*route)
            .arg("--help")
            .output()
            .unwrap_or_else(|error| panic!("run {route:?}: {error}"));
        assert!(
            output.status.success(),
            "documented route {route:?} is invalid: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if *route == ["config", "init"] {
            let help = String::from_utf8_lossy(&output.stdout);
            assert!(
                help.contains("Create a sparse default configuration"),
                "config init help must describe the sparse default"
            );
            assert!(
                !help.to_ascii_lowercase().contains("strict offline"),
                "config init help must not describe the default as strict offline"
            );
        }
    }
}
