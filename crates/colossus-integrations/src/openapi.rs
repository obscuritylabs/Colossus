use super::*;

/// Compile a bounded JSON OpenAPI 3 document into one canonical connection.
#[allow(clippy::too_many_arguments)]
pub fn compile_openapi(
    name: &str,
    document: &Value,
    base_url: Option<&str>,
    auth: IntegrationAuth,
    credential_reference: Option<String>,
    scopes: Vec<String>,
    connected_at: String,
    updated_at: String,
) -> Result<IntegrationConnection, StoreError> {
    validate_name(name)?;
    validate_auth(&auth)?;
    if matches!(auth, IntegrationAuth::Basic { .. }) {
        return Err(StoreError::Adapter(
            "OpenAPI imports do not accept named basic-auth credentials".into(),
        ));
    }
    validate_credential_reference(credential_reference.as_deref())?;
    let bytes = serde_json::to_vec(document).map_err(adapter)?;
    if bytes.len() > MAX_SCHEMA_BYTES {
        return Err(StoreError::Adapter("OpenAPI document exceeds 1 MiB".into()));
    }
    let root = document
        .as_object()
        .ok_or_else(|| StoreError::Adapter("OpenAPI document must be an object".into()))?;
    if !root
        .get("openapi")
        .and_then(Value::as_str)
        .is_some_and(|version| version.starts_with("3."))
    {
        return Err(StoreError::Adapter(
            "only OpenAPI 3.x JSON documents are supported".into(),
        ));
    }
    let info = root.get("info").and_then(Value::as_object);
    let title = info
        .and_then(|value| value.get("title"))
        .and_then(Value::as_str)
        .unwrap_or(name)
        .trim()
        .chars()
        .take(512)
        .collect::<String>();
    let description = info
        .and_then(|value| value.get("description"))
        .and_then(Value::as_str)
        .unwrap_or("Imported OpenAPI integration")
        .trim()
        .chars()
        .take(MAX_DESCRIPTION_BYTES)
        .collect::<String>();
    let base_url: String = base_url
        .map(str::to_owned)
        .or_else(|| {
            root.get("servers")
                .and_then(Value::as_array)
                .and_then(|servers| servers.first())
                .and_then(Value::as_object)
                .and_then(|server| server.get("url"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| StoreError::Adapter("OpenAPI base URL is required".into()))?;
    validate_base_url(&base_url)?;
    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| StoreError::Adapter("OpenAPI paths object is required".into()))?;
    let mut operations = Vec::new();
    let mut tool_names = BTreeSet::new();
    for (path, item) in paths {
        validate_api_path(path)?;
        let item = item
            .as_object()
            .ok_or_else(|| StoreError::Adapter("OpenAPI path item must be an object".into()))?;
        let path_parameters = item.get("parameters").and_then(Value::as_array);
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(operation) = item.get(method).and_then(Value::as_object) else {
                continue;
            };
            if operations.len() >= MAX_OPERATIONS {
                return Err(StoreError::Adapter(
                    "OpenAPI operation count exceeds 256".into(),
                ));
            }
            let compiled = compile_operation(name, method, path, path_parameters, operation)?;
            if !tool_names.insert(compiled.tool.name.clone()) {
                return Err(StoreError::Adapter(
                    "OpenAPI operation tool names must be unique".into(),
                ));
            }
            operations.push(compiled);
        }
    }
    if operations.is_empty() {
        return Err(StoreError::Adapter(
            "OpenAPI document contains no supported operations".into(),
        ));
    }
    let status = if auth_requires_credential(&auth) && credential_reference.is_none() {
        IntegrationStatus::PendingAuth
    } else {
        IntegrationStatus::Connected
    };
    let connection = IntegrationConnection {
        name: name.into(),
        kind: IntegrationKind::OpenApi,
        status,
        title,
        description,
        base_url,
        auth,
        credential_reference,
        credential_references: BTreeMap::new(),
        scopes,
        operations,
        manifest_sha256: format!("{:x}", Sha256::digest(&bytes)),
        connected_at,
        updated_at,
    };
    validate_connection(&connection)?;
    Ok(connection)
}
