use std::fmt;

pub(super) const USAGE: &str = "\
Colossus repository tasks

Usage:
  cargo xtask pre-commit
  cargo xtask dev
  cargo xtask pr [--base <git-revision>]
  cargo xtask check <component> [--base <git-revision>]

Components:
  rust          Formatting, structure, metadata, Clippy, and workspace tests
  sidecar       Managed Local sidecar contract tests
  sdk           Generated API compatibility and TypeScript, Python, and Go SDKs
  desktop       Desktop renderer audit, checks, tests, and build
  docs          Documentation build and verification
  dependencies  Root, fuzz, and desktop dependency policy
  workflows     CI classification contracts and workflow lint
";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Component {
    Rust,
    Sidecar,
    Sdk,
    Desktop,
    Docs,
    Dependencies,
    Workflows,
}

impl Component {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "rust" => Ok(Self::Rust),
            "sidecar" => Ok(Self::Sidecar),
            "sdk" => Ok(Self::Sdk),
            "desktop" => Ok(Self::Desktop),
            "docs" => Ok(Self::Docs),
            "dependencies" => Ok(Self::Dependencies),
            "workflows" => Ok(Self::Workflows),
            _ => Err(format!("unknown check component `{value}`\n\n{USAGE}")),
        }
    }
}

impl fmt::Display for Component {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Rust => "rust",
            Self::Sidecar => "sidecar",
            Self::Sdk => "sdk",
            Self::Desktop => "desktop",
            Self::Docs => "docs",
            Self::Dependencies => "dependencies",
            Self::Workflows => "workflows",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum Invocation {
    Help,
    PreCommit,
    Dev,
    Pr {
        base: Option<String>,
    },
    Check {
        component: Component,
        base: Option<String>,
    },
}

pub(super) fn parse(args: impl IntoIterator<Item = String>) -> Result<Invocation, String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(Invocation::Help);
    };
    match command.as_str() {
        "-h" | "--help" | "help" => no_more(args, Invocation::Help),
        "pre-commit" => no_more(args, Invocation::PreCommit),
        "dev" => no_more(args, Invocation::Dev),
        "pr" => Ok(Invocation::Pr {
            base: parse_base(args)?,
        }),
        "check" => {
            let component = args
                .next()
                .ok_or_else(|| format!("check requires a component\n\n{USAGE}"))?;
            Ok(Invocation::Check {
                component: Component::parse(&component)?,
                base: parse_base(args)?,
            })
        }
        _ => Err(format!("unknown task `{command}`\n\n{USAGE}")),
    }
}

fn no_more(
    mut args: impl Iterator<Item = String>,
    invocation: Invocation,
) -> Result<Invocation, String> {
    if let Some(argument) = args.next() {
        Err(format!("unexpected argument `{argument}`\n\n{USAGE}"))
    } else {
        Ok(invocation)
    }
}

fn parse_base(mut args: impl Iterator<Item = String>) -> Result<Option<String>, String> {
    let Some(flag) = args.next() else {
        return Ok(None);
    };
    if flag != "--base" {
        return Err(format!("unexpected argument `{flag}`\n\n{USAGE}"));
    }
    let base = args
        .next()
        .ok_or_else(|| format!("--base requires a git revision\n\n{USAGE}"))?;
    if args.next().is_some() {
        return Err(format!("unexpected arguments after `{base}`\n\n{USAGE}"));
    }
    Ok(Some(base))
}

#[cfg(test)]
mod tests {
    use super::{Component, Invocation, parse};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_tiered_and_component_commands() {
        assert_eq!(parse(strings(&[])).unwrap(), Invocation::Help);
        assert_eq!(
            parse(strings(&["pre-commit"])).unwrap(),
            Invocation::PreCommit
        );
        assert_eq!(parse(strings(&["dev"])).unwrap(), Invocation::Dev);
        assert_eq!(
            parse(strings(&["pr", "--base", "origin/main"])).unwrap(),
            Invocation::Pr {
                base: Some("origin/main".to_owned())
            }
        );
        assert_eq!(
            parse(strings(&["check", "sdk", "--base", "abc123"])).unwrap(),
            Invocation::Check {
                component: Component::Sdk,
                base: Some("abc123".to_owned())
            }
        );
    }

    #[test]
    fn rejects_unknown_or_incomplete_commands() {
        assert!(
            parse(strings(&["check"]))
                .unwrap_err()
                .contains("component")
        );
        assert!(
            parse(strings(&["check", "unknown"]))
                .unwrap_err()
                .contains("unknown check component")
        );
        assert!(
            parse(strings(&["pr", "--base"]))
                .unwrap_err()
                .contains("requires a git revision")
        );
        assert!(
            parse(strings(&["dev", "extra"]))
                .unwrap_err()
                .contains("unexpected argument")
        );
    }
}
