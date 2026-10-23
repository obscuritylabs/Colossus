use std::fmt;

pub(super) const USAGE: &str = "\
Colossus repository tasks

Usage:
  cargo xtask pre-commit
  cargo xtask dev
  cargo xtask quick <check|clippy|test> -p <package>... [-- <cargo-args>...]
  cargo xtask pr [--base <git-revision>]
  cargo xtask check <component> [--base <git-revision>]
  cargo xtask desktop prepare --profile <debug|release> [--target <triple>]

Components:
  rust          Formatting, structure, metadata, Clippy, and workspace tests
  sidecar       Managed Local sidecar contract tests
  sdk           Generated API compatibility and TypeScript, Python, and Go SDKs
  desktop       Desktop renderer audit, checks, tests, and build
  web           Cloud web renderer audit, checks, tests, and build
  helm          Control Plane chart lint, render matrix, and offline packaging
  docs          Documentation build and verification
  dependencies  Root, fuzz, Desktop, and standalone native dependency policy
  workflows     CI classification contracts and workflow lint

Quick checks require explicit package selection and do not replace completion gates.
Pass Cargo target selectors after --, for example: quick test -p colossus-policy -- --lib
";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Component {
    Rust,
    Sidecar,
    Sdk,
    Desktop,
    Web,
    Helm,
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
            "web" => Ok(Self::Web),
            "helm" => Ok(Self::Helm),
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
            Self::Web => "web",
            Self::Helm => "helm",
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
    Quick {
        command: RustCommand,
        packages: Vec<String>,
        arguments: Vec<String>,
    },
    Pr {
        base: Option<String>,
    },
    Check {
        component: Component,
        base: Option<String>,
    },
    DesktopPrepare {
        profile: DesktopProfile,
        target: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RustCommand {
    Check,
    Clippy,
    Test,
}

impl fmt::Display for RustCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Check => "check",
            Self::Clippy => "clippy",
            Self::Test => "test",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DesktopProfile {
    Debug,
    Release,
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
        "quick" => parse_quick(args),
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
        "desktop" => parse_desktop(args),
        _ => Err(format!("unknown task `{command}`\n\n{USAGE}")),
    }
}

fn parse_quick(mut args: impl Iterator<Item = String>) -> Result<Invocation, String> {
    let command = match args.next().as_deref() {
        Some("check") => RustCommand::Check,
        Some("clippy") => RustCommand::Clippy,
        Some("test") => RustCommand::Test,
        _ => return Err(format!("quick requires check, clippy, or test\n\n{USAGE}")),
    };
    let mut packages = Vec::new();
    let mut arguments = Vec::new();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "-p" | "--package" => {
                let package = args
                    .next()
                    .filter(|value| !value.is_empty() && !value.starts_with('-'))
                    .ok_or_else(|| format!("{flag} requires a package\n\n{USAGE}"))?;
                packages.push(package);
            }
            "--" => {
                arguments.extend(args);
                break;
            }
            _ => return Err(format!("unexpected quick argument `{flag}`\n\n{USAGE}")),
        }
    }
    if packages.is_empty() {
        return Err(format!(
            "quick requires at least one -p <package>\n\n{USAGE}"
        ));
    }
    Ok(Invocation::Quick {
        command,
        packages,
        arguments,
    })
}

fn parse_desktop(mut args: impl Iterator<Item = String>) -> Result<Invocation, String> {
    if args.next().as_deref() != Some("prepare") {
        return Err(format!("desktop requires `prepare`\n\n{USAGE}"));
    }
    let mut profile = None;
    let mut target = None;
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("{flag} requires a value\n\n{USAGE}"))?;
        match flag.as_str() {
            "--profile" if profile.is_none() => {
                profile = Some(match value.as_str() {
                    "debug" => DesktopProfile::Debug,
                    "release" => DesktopProfile::Release,
                    _ => return Err(format!("invalid desktop profile `{value}`\n\n{USAGE}")),
                });
            }
            "--target" if target.is_none() && valid_target(&value) => target = Some(value),
            "--target" => return Err(format!("invalid desktop target `{value}`\n\n{USAGE}")),
            _ => return Err(format!("unexpected desktop argument `{flag}`\n\n{USAGE}")),
        }
    }
    Ok(Invocation::DesktopPrepare {
        profile: profile.ok_or_else(|| format!("desktop prepare requires --profile\n\n{USAGE}"))?,
        target,
    })
}

fn valid_target(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
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
    use super::{Component, DesktopProfile, Invocation, RustCommand, parse};

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
        assert_eq!(
            parse(strings(&[
                "desktop",
                "prepare",
                "--profile",
                "release",
                "--target",
                "x86_64-pc-windows-msvc"
            ]))
            .unwrap(),
            Invocation::DesktopPrepare {
                profile: DesktopProfile::Release,
                target: Some("x86_64-pc-windows-msvc".to_owned())
            }
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

    #[test]
    fn quick_requires_explicit_packages_and_preserves_cargo_arguments() {
        assert_eq!(
            parse(strings(&[
                "quick",
                "test",
                "-p",
                "colossus-policy",
                "--package",
                "colossus-access",
                "--",
                "--lib",
                "policy::tests",
                "--",
                "--nocapture"
            ]))
            .unwrap(),
            Invocation::Quick {
                command: RustCommand::Test,
                packages: strings(&["colossus-policy", "colossus-access"]),
                arguments: strings(&["--lib", "policy::tests", "--", "--nocapture"]),
            }
        );
        for invalid in [
            strings(&["quick"]),
            strings(&["quick", "build", "-p", "xtask"]),
            strings(&["quick", "check"]),
            strings(&["quick", "check", "--", "--workspace"]),
            strings(&["quick", "check", "-p"]),
            strings(&["quick", "check", "-p", "--workspace"]),
            strings(&["quick", "check", "-p", ""]),
            strings(&["quick", "check", "--workspace"]),
        ] {
            assert!(parse(invalid.clone()).is_err(), "accepted {invalid:?}");
        }
        for command in ["check", "clippy"] {
            assert!(parse(strings(&["quick", command, "-p", "xtask"])).is_ok());
        }
    }
}
