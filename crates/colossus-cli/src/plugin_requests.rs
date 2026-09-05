//! Translate CLI/TUI syntax into shared runtime operator requests.

use super::*;
use colossus_contracts::{PluginInstallSource as Source, PluginManagementRequest as Request};

impl PluginsAction {
    pub(super) fn request(&self) -> Result<Request, String> {
        Ok(match self {
            Self::List { .. } => Request::Inventory,
            Self::Show { name } => Request::Show { name: name.clone() },
            Self::Validate { directory } => Request::Validate {
                path: directory.display().to_string(),
            },
            Self::Verify {
                path,
                digest,
                trust_profile,
            } => Request::Verify {
                path: path.display().to_string(),
                digest: digest.clone(),
                trust_profile: trust_profile.clone(),
            },
            Self::Install {
                directory,
                reference,
                layout,
                archive,
                digest,
                registry,
                trust_profile,
            } => {
                if reference.is_none() && registry.is_some() {
                    return Err("--registry requires --reference".into());
                }
                let source = if let Some(path) = directory {
                    if digest.is_some() {
                        return Err(
                            "--digest selects an OCI candidate, not a source directory".into()
                        );
                    }
                    Source::Directory {
                        path: path.display().to_string(),
                    }
                } else if let Some(path) = layout {
                    Source::Layout {
                        path: path.display().to_string(),
                        digest: digest.clone(),
                    }
                } else if let Some(path) = archive {
                    Source::Archive {
                        path: path.display().to_string(),
                        digest: digest.clone(),
                    }
                } else if let Some(reference) = reference {
                    if digest.is_some() {
                        return Err("pin the registry reference with @sha256:DIGEST".into());
                    }
                    Source::Reference {
                        registry: registry.clone().ok_or("--reference requires --registry")?,
                        reference: reference.clone(),
                    }
                } else {
                    return Err("select one plugin installation source".into());
                };
                Request::Install {
                    source,
                    trust_profile: trust_profile.clone(),
                }
            }
            Self::Enable {
                name,
                digest,
                allow_untrusted,
            } => Request::Enable {
                name: name.clone(),
                digest: digest.clone(),
                allow_untrusted: *allow_untrusted,
            },
            Self::Disable { name } => Request::Disable { name: name.clone() },
            Self::Update {
                name,
                reference,
                registry,
            } => Request::Update {
                name: name.clone(),
                reference: reference.clone(),
                registry: registry.clone(),
            },
            Self::Uninstall {
                name,
                digest,
                purge_data,
            } => Request::Uninstall {
                name: name.clone(),
                digest: digest.clone(),
                purge_data: *purge_data,
            },
            Self::Gc => Request::Gc,
            Self::Package { directory, output } => Request::Package {
                directory: directory.display().to_string(),
                output: output.display().to_string(),
            },
            Self::Push {
                layout,
                reference,
                registry,
            } => Request::Push {
                layout: layout.display().to_string(),
                reference: reference.clone(),
                registry: registry.clone(),
            },
            Self::Pull {
                reference,
                output,
                registry,
            } => Request::Pull {
                reference: reference.clone(),
                output: output.display().to_string(),
                registry: registry.clone(),
            },
            Self::Export { name, output } => Request::Export {
                name: name.clone(),
                output: output.display().to_string(),
            },
        })
    }
}
