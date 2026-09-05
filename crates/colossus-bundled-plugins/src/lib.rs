//! Release-bound, offline bootstrap content for the Colossus core Agent Plugin.

use colossus_plugins::{BuiltPluginArtifact, build_plugin_artifact_from_files};
use colossus_ports::StoreError;

mod content {
    include!(concat!(env!("OUT_DIR"), "/content.rs"));
}

/// Build the canonical core OCI artifact from bytes compiled into this executable.
pub fn core_artifact() -> Result<BuiltPluginArtifact, StoreError> {
    let artifact = build_plugin_artifact_from_files(content::CORE_FILES)?;
    if artifact
        .parsed_manifest
        .annotations
        .get("org.opencontainers.image.version")
        .map(String::as_str)
        != Some(env!("CARGO_PKG_VERSION"))
    {
        return Err(StoreError::Verification(
            "bundled core version does not match Colossus".into(),
        ));
    }
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_core_matches_directory_packaging_and_has_four_valid_skills() {
        let embedded = core_artifact().expect("embedded artifact");
        let source =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-plugins/colossus");
        let directory =
            colossus_plugins::build_plugin_artifact(&source).expect("directory artifact");
        assert_eq!(embedded.manifest_digest, directory.manifest_digest);
        assert_eq!(embedded.layer, directory.layer);
        let temporary = tempfile::tempdir().expect("temporary");
        let root = temporary.path().join("content");
        colossus_plugins::extract_plugin_artifact(&embedded, &root).expect("extract");
        let record = colossus_plugins::load_plugin(&root).expect("load");
        assert!(record.diagnostics.is_empty(), "{:?}", record.diagnostics);
        assert_eq!(
            record
                .skills
                .iter()
                .map(|skill| skill.id.as_str())
                .collect::<Vec<_>>(),
            [
                "colossus/coding",
                "colossus/offline-dev",
                "colossus/plugin-authoring",
                "colossus/security-review"
            ]
        );
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
            let artifact =
                colossus_plugins::build_plugin_artifact(&source).expect("template artifact");
            let temp = tempfile::tempdir().expect("temporary");
            let extracted = temp.path().join("plugin");
            colossus_plugins::extract_plugin_artifact(&artifact, &extracted)
                .expect("extract template");
            assert!(
                colossus_plugins::load_plugin(&extracted)
                    .expect("load template")
                    .diagnostics
                    .is_empty()
            );
        }
    }
}
