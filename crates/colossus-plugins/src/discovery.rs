use super::*;

/// Load and validate a complete Agent Plugin directory.
pub fn load_plugin(root: &Path) -> Result<AgentPluginRecord, StoreError> {
    let canonical_root = canonical_plugin_root(root)?;
    let (manifest, mut diagnostics) = load_manifest(&canonical_root)?;
    let skills = load_skills(&canonical_root, &manifest.name, &mut diagnostics)?;
    let mcp_servers = load_mcp(&canonical_root, &manifest, &mut diagnostics)?;
    let content_sha256 = hash_plugin_tree(&canonical_root)?.2;
    let installation = PluginInstallation {
        origin: colossus_contracts::PluginOrigin::Installed,
        manifest,
        digest: format!("sha256:{content_sha256}"),
        source: canonical_root.display().to_string(),
        root: canonical_root.display().to_string(),
        status: PluginStatus::Disabled,
        trust: PluginTrustEvidence {
            trusted: false,
            profile: None,
            signer: None,
            method: "local-directory".into(),
        },
        installed_at: String::new(),
        updated_at: String::new(),
    };
    Ok(AgentPluginRecord {
        installation,
        skills,
        mcp_servers,
        diagnostics,
    })
}

/// Validate one Agent Plugin directory without returning instructions or resource bodies.
pub fn validate_plugin(root: &Path) -> Result<PluginValidation, StoreError> {
    let record = load_plugin(root)?;
    let (file_count, total_bytes, content_sha256) =
        hash_plugin_tree(Path::new(&record.installation.root))?;
    Ok(PluginValidation {
        manifest: record.installation.manifest,
        file_count,
        total_bytes,
        content_sha256,
        diagnostics: record.diagnostics,
    })
}

pub(crate) fn canonical_plugin_root(root: &Path) -> Result<PathBuf, StoreError> {
    Ok(ReadRoot::bind(root)?.path().to_owned())
}

pub(crate) fn load_manifest(
    root: &Path,
) -> Result<(AgentPluginManifest, Vec<PluginComponentDiagnostic>), StoreError> {
    let bytes = read_contained(root, Path::new("plugin.json"), MAX_MANIFEST_BYTES)?;
    parse_plugin_manifest(&bytes)
}

pub(crate) fn parse_plugin_manifest(
    bytes: &[u8],
) -> Result<(AgentPluginManifest, Vec<PluginComponentDiagnostic>), StoreError> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(StoreError::Adapter("plugin.json exceeds 1 MiB".into()));
    }
    let mut value: Value = serde_json::from_slice(bytes).map_err(adapter)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| StoreError::Adapter("plugin.json must be a JSON object".into()))?;
    let allowed = [
        "$schema",
        "name",
        "version",
        "description",
        "author",
        "homepage",
        "repository",
        "license",
        "keywords",
        "extensions",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let unknown = object
        .keys()
        .filter(|key| !allowed.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    for key in unknown {
        object.remove(&key);
        diagnostics.push(component_diagnostic(
            PluginComponentKind::Plugin,
            None,
            "unknown_manifest_field",
            format!("ignored unknown plugin.json field {key}"),
        ));
    }
    if object
        .get("extensions")
        .is_some_and(|extensions| !extensions.is_object())
    {
        object.remove("extensions");
        diagnostics.push(component_diagnostic(
            PluginComponentKind::Plugin,
            None,
            "invalid_extensions_ignored",
            "ignored non-object plugin.json extensions field",
        ));
    }
    validate_plugin_schema(&value)?;
    let manifest: AgentPluginManifest = serde_json::from_value(value).map_err(adapter)?;
    if manifest.schema != AGENT_PLUGIN_SCHEMA_V1 {
        return Err(StoreError::Adapter(format!(
            "unsupported Agent Plugins schema {}",
            manifest.schema
        )));
    }
    Ok((manifest, diagnostics))
}

fn validate_plugin_schema(value: &Value) -> Result<(), StoreError> {
    let validator = super::schema::plugin_validator()?;
    let errors = validator
        .iter_errors(value)
        .take(8)
        .map(|error| {
            format!(
                "field {} violates schema rule {}",
                error.instance_path, error.schema_path
            )
        })
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(StoreError::Adapter(format!(
            "plugin.json does not conform to Agent Plugins v1: {}",
            errors.join("; ")
        )))
    }
}
