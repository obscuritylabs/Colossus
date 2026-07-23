use std::collections::BTreeSet;

use crate::repository::Repository;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Classification {
    pub(super) rust: bool,
    pub(super) docs: bool,
    pub(super) dependencies: bool,
    pub(super) sdk: bool,
    pub(super) desktop: bool,
}

impl Classification {
    fn parse(output: &str) -> Result<Self, String> {
        let mut classification = Self::default();
        let mut seen = BTreeSet::new();
        for line in output.lines() {
            let (name, raw_value) = line
                .split_once('=')
                .ok_or_else(|| format!("invalid change classification line `{line}`"))?;
            let value = match raw_value {
                "true" => true,
                "false" => false,
                _ => return Err(format!("invalid boolean in change classification `{line}`")),
            };
            if !seen.insert(name) {
                return Err(format!("duplicate change classification field `{name}`"));
            }
            match name {
                "rust_required" => classification.rust = value,
                "docs_required" => classification.docs = value,
                "dependency_required" => classification.dependencies = value,
                "sdk_required" => classification.sdk = value,
                "desktop_required" => classification.desktop = value,
                _ => return Err(format!("unknown change classification field `{name}`")),
            }
        }
        for required in [
            "rust_required",
            "docs_required",
            "dependency_required",
            "sdk_required",
            "desktop_required",
        ] {
            if !seen.contains(required) {
                return Err(format!("change classification is missing `{required}`"));
            }
        }
        Ok(classification)
    }

    pub(super) fn summary(self) -> String {
        let mut selected = Vec::new();
        if self.rust {
            selected.push("rust");
        }
        if self.sdk {
            selected.push("sdk");
        }
        if self.desktop {
            selected.push("desktop");
        }
        if self.docs {
            selected.push("docs");
        }
        if self.dependencies {
            selected.push("dependencies");
        }
        selected.join(", ")
    }
}

pub(super) fn resolve_base(
    repository: &Repository,
    requested: Option<String>,
) -> Result<String, String> {
    if let Some(base) = requested {
        let revision = format!("{base}^{{commit}}");
        repository
            .task("git")
            .args(["rev-parse", "--verify", &revision])
            .quiet_stdout()
            .run()
            .map_err(|_| format!("base revision `{base}` is not available"))?;
        return Ok(base);
    }

    for candidate in ["origin/main", "main"] {
        if let Some(output) = repository
            .task("git")
            .args(["merge-base", "HEAD", candidate])
            .optional_output()?
        {
            let base = output.trim();
            if !base.is_empty() {
                return Ok(base.to_owned());
            }
        }
    }
    Err(
        "could not determine a PR base; fetch `origin/main` or pass `--base <git-revision>`"
            .to_owned(),
    )
}

pub(super) fn classify_changes(
    repository: &Repository,
    base: &str,
) -> Result<Option<Classification>, String> {
    let mut paths = BTreeSet::new();
    let changed = repository
        .task("git")
        .args([
            "-c",
            "core.quotePath=false",
            "diff",
            "--name-only",
            "--diff-filter=ACDMRTUXB",
            base,
            "--",
        ])
        .output()?;
    paths.extend(
        changed
            .lines()
            .filter(|path| !path.is_empty())
            .map(str::to_owned),
    );

    let untracked = repository
        .task("git")
        .args([
            "-c",
            "core.quotePath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
        ])
        .output()?;
    paths.extend(
        untracked
            .lines()
            .filter(|path| !path.is_empty())
            .map(str::to_owned),
    );
    if paths.is_empty() {
        return Ok(None);
    }

    let output = repository
        .task("./scripts/ci/classify-changes.sh")
        .args(paths)
        .output()?;
    Classification::parse(&output).map(Some)
}

#[cfg(test)]
mod tests {
    use super::Classification;

    #[test]
    fn parses_all_classifier_outputs() {
        let parsed = Classification::parse(
            "rust_required=true\n\
             docs_required=false\n\
             dependency_required=true\n\
             sdk_required=false\n\
             desktop_required=true\n",
        )
        .unwrap();
        assert_eq!(
            parsed,
            Classification {
                rust: true,
                docs: false,
                dependencies: true,
                sdk: false,
                desktop: true,
            }
        );
        assert_eq!(parsed.summary(), "rust, desktop, dependencies");
    }

    #[test]
    fn classification_fails_closed_on_schema_drift() {
        assert!(
            Classification::parse(
                "rust_required=true\n\
                 docs_required=false\n\
                 dependency_required=false\n\
                 sdk_required=false\n"
            )
            .unwrap_err()
            .contains("desktop_required")
        );
        assert!(
            Classification::parse(
                "rust_required=true\n\
                 docs_required=false\n\
                 dependency_required=false\n\
                 sdk_required=false\n\
                 desktop_required=false\n\
                 mobile_required=true\n"
            )
            .unwrap_err()
            .contains("unknown")
        );
    }
}
