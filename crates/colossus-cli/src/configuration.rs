use super::*;

pub(super) fn integration_auth(
    mode: IntegrationAuthMode,
    header: String,
    scheme: Option<String>,
) -> IntegrationAuth {
    match mode {
        IntegrationAuthMode::None => IntegrationAuth::None,
        IntegrationAuthMode::Bearer => IntegrationAuth::Bearer {
            header,
            scheme: scheme.unwrap_or_else(|| "Bearer".into()),
        },
        IntegrationAuthMode::ApiKey => IntegrationAuth::ApiKey { header, scheme },
        IntegrationAuthMode::Basic => IntegrationAuth::Basic { header },
        IntegrationAuthMode::ServiceAccount => IntegrationAuth::ServiceAccount { header },
    }
}

pub(super) async fn parse_json_argument(
    runtime: &Runtime,
    source: &str,
) -> Result<Value, Box<dyn Error>> {
    let document = if let Some(path) = source.strip_prefix('@') {
        runtime.read_text_file(path).await?
    } else {
        source.to_owned()
    };
    Ok(serde_json::from_str(&document)?)
}

pub(super) fn init_config(
    path: &Path,
    development: bool,
    from: Option<&Path>,
    access_profile: AccessProfile,
    sandbox_profile: Option<SandboxProfile>,
) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("refusing to overwrite {}", path.display()).into());
    }
    if !development && from.is_some() {
        return Err("--from requires --development".into());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }
    let state = parent.join(if development {
        "state.dev.redb"
    } else {
        "state.redb"
    });
    let anchor = parent.join("secure-anchor.dev.json");
    if development && (state.exists() || anchor.exists()) {
        return Err(format!(
            "refusing to create {} while isolated development state or anchor already exists; restore the matching config or remove both {} and {}",
            path.display(),
            state.display(),
            anchor.display()
        )
        .into());
    }
    let mut config = if let Some(source) = from {
        RuntimeConfig::from_path(source)?
    } else {
        RuntimeConfig::offline_template(&state)
    };
    config.set_access_profile(access_profile);
    config.set_sandbox_profile(
        sandbox_profile
            .unwrap_or_else(|| {
                if access_profile == AccessProfile::Development {
                    SandboxProfile::WorkspaceDevelopment
                } else {
                    SandboxProfile::OfflineDefault
                }
            })
            .as_str(),
    );
    let config = if development {
        config.with_isolated_development_storage(state, anchor)
    } else {
        config
    };
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    destination.write_all(config.to_yaml()?.as_bytes())?;
    println!("created {}", path.display());
    Ok(())
}
