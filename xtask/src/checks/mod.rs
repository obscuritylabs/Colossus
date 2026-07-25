mod desktop;
mod rust;
mod sdk;
mod surfaces;

use crate::{
    cli::{Component, Invocation},
    repository::Repository,
    selection,
};

pub(super) fn run(repository: &Repository, invocation: Invocation) -> Result<(), String> {
    match invocation {
        Invocation::Help => unreachable!("help is handled before repository discovery"),
        Invocation::PreCommit => rust::pre_commit(repository),
        Invocation::Dev => rust::dev(repository),
        Invocation::Pr { base } => pr(repository, base),
        Invocation::Check { component, base } => component_check(repository, component, base),
        Invocation::DesktopPrepare { profile, target } => {
            desktop::prepare(repository, profile, target.as_deref())
        }
    }
}

fn pr(repository: &Repository, requested_base: Option<String>) -> Result<(), String> {
    let base = selection::resolve_base(repository, requested_base)?;
    surfaces::workflows(repository)?;
    let Some(classification) = selection::classify_changes(repository, &base)? else {
        eprintln!("No changes found relative to {base}; running the cheap local checks only.");
        return rust::pre_commit(repository);
    };
    eprintln!(
        "Selected checks relative to {base}: {}",
        classification.summary()
    );

    if classification.rust {
        rust::full(repository)?;
    }
    if classification.sdk {
        sdk::check(repository, &base)?;
    }
    if classification.desktop {
        surfaces::sidecar(repository)?;
        surfaces::desktop(repository)?;
    }
    if classification.docs {
        surfaces::docs(repository)?;
    }
    if classification.dependencies {
        surfaces::dependencies(repository)?;
    }
    eprintln!("All selected pre-PR checks passed.");
    Ok(())
}

fn component_check(
    repository: &Repository,
    component: Component,
    requested_base: Option<String>,
) -> Result<(), String> {
    if component != Component::Sdk && requested_base.is_some() {
        return Err(format!(
            "--base is only valid with `pr` or `check sdk`, not `{component}`"
        ));
    }
    match component {
        Component::Rust => rust::full(repository),
        Component::Sidecar => surfaces::sidecar(repository),
        Component::Sdk => {
            let base = selection::resolve_base(repository, requested_base)?;
            sdk::check(repository, &base)
        }
        Component::Desktop => surfaces::desktop(repository),
        Component::Docs => surfaces::docs(repository),
        Component::Dependencies => surfaces::dependencies(repository),
        Component::Workflows => surfaces::workflows(repository),
    }
}
