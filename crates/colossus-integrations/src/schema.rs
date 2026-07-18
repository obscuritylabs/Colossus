use super::*;

pub(super) fn compile_operation(
    integration: &str,
    method: &str,
    path: &str,
    path_parameters: Option<&Vec<Value>>,
    operation: &Map<String, Value>,
) -> Result<IntegrationOperation, StoreError> {
    let operation_id = operation
        .get("operationId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{method}_{}", path.trim_matches('/').replace('/', "_")));
    let segment = sanitize_segment(&operation_id)?;
    let tool_name = format!("openapi.{integration}.{segment}");
    let description = operation
        .get("description")
        .or_else(|| operation.get("summary"))
        .and_then(Value::as_str)
        .unwrap_or("Imported OpenAPI operation")
        .chars()
        .take(MAX_DESCRIPTION_BYTES)
        .collect::<String>();
    let mut properties = Map::new();
    let mut required = BTreeSet::<String>::new();
    let mut path_names = Vec::new();
    let mut query_names = Vec::new();
    for parameter in path_parameters.into_iter().flatten().chain(
        operation
            .get("parameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    ) {
        let parameter = parameter.as_object().ok_or_else(|| {
            StoreError::Adapter("OpenAPI parameters must be inline objects".into())
        })?;
        let name = parameter
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| StoreError::Adapter("OpenAPI parameter name is required".into()))?;
        validate_argument_name(name)?;
        let location = parameter
            .get("in")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(location, "path" | "query") {
            continue;
        }
        properties.insert(
            name.into(),
            simple_schema(
                parameter.get("schema").unwrap_or(&json!({"type":"string"})),
                0,
            )?,
        );
        if location == "path" {
            path_names.push(name.into());
            required.insert(name.into());
        } else {
            query_names.push(name.into());
            if parameter.get("required") == Some(&Value::Bool(true)) {
                required.insert(name.into());
            }
        }
    }
    for name in &path_names {
        if !path.contains(&format!("{{{name}}}")) {
            return Err(StoreError::Adapter(format!(
                "OpenAPI path parameter {name} is absent from its template"
            )));
        }
    }
    let mut accepts_body = false;
    if let Some(body) = operation.get("requestBody").and_then(Value::as_object) {
        let schema = body
            .get("content")
            .and_then(Value::as_object)
            .and_then(|content| content.get("application/json"))
            .and_then(Value::as_object)
            .and_then(|media| media.get("schema"))
            .map_or_else(
                || Ok(json!({"type":"object"})),
                |schema| simple_schema(schema, 0),
            )?;
        properties.insert("body".into(), schema);
        accepts_body = true;
        if body.get("required") == Some(&Value::Bool(true)) {
            required.insert("body".into());
        }
    }
    let input_schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });
    jsonschema::validator_for(&input_schema).map_err(adapter)?;
    Ok(IntegrationOperation {
        tool: ToolSpec {
            name: tool_name.clone(),
            description,
            input_schema,
            effect_action: Some(tool_name.clone()),
            capability: Some("integration.invoke".into()),
            max_output_bytes: 64_000,
        },
        operation_id,
        method: method.to_ascii_uppercase(),
        path: path.into(),
        path_parameters: path_names,
        query_parameters: query_names,
        accepts_body,
    })
}

pub(super) fn simple_schema(value: &Value, depth: usize) -> Result<Value, StoreError> {
    if depth > 8 {
        return Err(StoreError::Adapter(
            "OpenAPI schema nesting exceeds 8".into(),
        ));
    }
    let object = value
        .as_object()
        .ok_or_else(|| StoreError::Adapter("OpenAPI schemas must be objects".into()))?;
    if object.contains_key("$ref") {
        return Err(StoreError::Adapter(
            "OpenAPI schema references are not supported by the bounded importer".into(),
        ));
    }
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("object");
    if !matches!(
        kind,
        "string" | "integer" | "number" | "boolean" | "array" | "object"
    ) {
        return Err(StoreError::Adapter(
            "unsupported OpenAPI schema type".into(),
        ));
    }
    let mut schema = Map::new();
    schema.insert("type".into(), Value::String(kind.into()));
    for key in [
        "description",
        "enum",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "pattern",
        "format",
        "default",
    ] {
        if let Some(value) = object.get(key) {
            schema.insert(key.into(), value.clone());
        }
    }
    if kind == "array" {
        schema.insert(
            "items".into(),
            simple_schema(
                object.get("items").unwrap_or(&json!({"type":"string"})),
                depth + 1,
            )?,
        );
        schema.insert("maxItems".into(), json!(1_000));
    }
    if kind == "object" {
        let mut properties = Map::new();
        if let Some(values) = object.get("properties").and_then(Value::as_object) {
            if values.len() > 256 {
                return Err(StoreError::Adapter(
                    "OpenAPI object property count exceeds 256".into(),
                ));
            }
            for (name, value) in values {
                validate_argument_name(name)?;
                properties.insert(name.clone(), simple_schema(value, depth + 1)?);
            }
        }
        schema.insert("properties".into(), Value::Object(properties));
        schema.insert("additionalProperties".into(), Value::Bool(false));
        if let Some(required) = object.get("required") {
            schema.insert("required".into(), required.clone());
        }
    }
    Ok(Value::Object(schema))
}

pub(super) fn validate_connection(connection: &IntegrationConnection) -> Result<(), StoreError> {
    validate_name(&connection.name)?;
    validate_base_url(&connection.base_url)?;
    validate_auth(&connection.auth)?;
    validate_credential_reference(connection.credential_reference.as_deref())?;
    for reference in connection.credential_references.values() {
        validate_credential_reference(Some(reference))?;
    }
    if connection.title.trim().is_empty()
        || connection.title.len() > 512
        || connection.description.len() > MAX_DESCRIPTION_BYTES
        || connection.operations.is_empty()
        || connection.operations.len() > MAX_OPERATIONS
        || connection.manifest_sha256.len() != 64
        || connection.connected_at.is_empty()
        || connection.updated_at.is_empty()
        || connection.scopes.len() > 128
        || connection
            .scopes
            .iter()
            .any(|scope| scope.is_empty() || scope.len() > 512)
    {
        return Err(StoreError::Adapter(
            "integration connection violates identity or size bounds".into(),
        ));
    }
    if connection.status == IntegrationStatus::Connected
        && !credentials_satisfy_auth(
            &connection.auth,
            connection.credential_reference.as_deref(),
            &connection.credential_references,
        )
    {
        return Err(StoreError::Adapter(
            "connected authenticated integration requires a credential reference".into(),
        ));
    }
    if !auth_requires_credential(&connection.auth)
        && (connection.credential_reference.is_some()
            || !connection.credential_references.is_empty())
    {
        return Err(StoreError::Adapter(
            "auth-none integrations cannot retain a credential reference".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for operation in &connection.operations {
        let valid_prefix = match connection.kind {
            IntegrationKind::OpenApi => format!("openapi.{}.", connection.name),
            IntegrationKind::Native => format!("{}.", connection.name),
            IntegrationKind::Mcp => format!("mcp.{}.", connection.name),
        };
        if !names.insert(operation.tool.name.as_str())
            || operation.tool.effect_action.as_deref() != Some(&operation.tool.name)
            || operation.tool.capability.as_deref() != Some("integration.invoke")
            || !operation.tool.name.starts_with(&valid_prefix)
            || !matches!(
                operation.method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
            )
        {
            return Err(StoreError::Adapter(
                "integration operation identity is invalid".into(),
            ));
        }
        jsonschema::validator_for(&operation.tool.input_schema).map_err(adapter)?;
    }
    Ok(())
}

pub(super) fn validate_name(name: &str) -> Result<(), StoreError> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name.bytes().enumerate().all(|(index, byte)| {
            if index == 0 {
                byte.is_ascii_lowercase()
            } else {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            }
        });
    if valid {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "integration names must be bounded lowercase identifiers".into(),
        ))
    }
}

pub(super) fn validate_argument_name(name: &str) -> Result<(), StoreError> {
    if !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "OpenAPI argument name is invalid".into(),
        ))
    }
}

pub(super) fn sanitize_segment(value: &str) -> Result<String, StoreError> {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() || value.len() > 128 {
        Err(StoreError::Adapter(
            "OpenAPI operation id is invalid".into(),
        ))
    } else {
        Ok(value)
    }
}

pub(super) fn validate_api_path(path: &str) -> Result<(), StoreError> {
    if path.starts_with('/')
        && path.len() <= 4_096
        && !path.contains(['?', '#', '\\'])
        && path
            .split('/')
            .all(|component| !matches!(component, "." | ".."))
    {
        Ok(())
    } else {
        Err(StoreError::Adapter("invalid OpenAPI operation path".into()))
    }
}

pub(super) fn validate_base_url(value: &str) -> Result<(), StoreError> {
    let url = Url::parse(value).map_err(adapter)?;
    if matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
    {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "integration base URL requires HTTP(S), a host, and no credentials/query/fragment"
                .into(),
        ))
    }
}

pub(super) fn validate_auth(auth: &IntegrationAuth) -> Result<(), StoreError> {
    match auth {
        IntegrationAuth::None => Ok(()),
        IntegrationAuth::Bearer { header, scheme }
            if valid_header(header) && !scheme.is_empty() && scheme.len() <= 64 =>
        {
            Ok(())
        }
        IntegrationAuth::ApiKey { header, scheme }
            if valid_header(header)
                && scheme
                    .as_ref()
                    .is_none_or(|value| !value.is_empty() && value.len() <= 64) =>
        {
            Ok(())
        }
        IntegrationAuth::Basic { header } if valid_header(header) => Ok(()),
        IntegrationAuth::ServiceAccount { header } if valid_header(header) => Ok(()),
        _ => Err(StoreError::Adapter(
            "integration auth header or scheme is invalid".into(),
        )),
    }
}

pub(super) fn valid_header(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && HeaderName::from_bytes(value.as_bytes()).is_ok()
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "host" | "content-length"
        )
}

pub(super) fn validate_credential_reference(value: Option<&str>) -> Result<(), StoreError> {
    if value.is_none_or(valid_environment_reference) {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "integration credentials must use env:VARIABLE references".into(),
        ))
    }
}

pub(super) fn valid_environment_reference(value: &str) -> bool {
    value.strip_prefix("env:").is_some_and(|name| {
        let mut bytes = name.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    })
}

pub(super) fn auth_requires_credential(auth: &IntegrationAuth) -> bool {
    !matches!(auth, IntegrationAuth::None)
}

pub(super) fn credentials_satisfy_auth(
    auth: &IntegrationAuth,
    credential_reference: Option<&str>,
    credential_references: &BTreeMap<String, String>,
) -> bool {
    match auth {
        IntegrationAuth::None => credential_reference.is_none() && credential_references.is_empty(),
        IntegrationAuth::Basic { .. } => {
            credential_reference.is_none()
                && credential_references.len() == 2
                && credential_references.contains_key("username")
                && credential_references.contains_key("password")
        }
        _ => credential_reference.is_some() && credential_references.is_empty(),
    }
}

pub(super) fn validate_native_auth(
    name: &str,
    auth: &IntegrationAuth,
    credential_reference: Option<&str>,
    credential_references: &BTreeMap<String, String>,
) -> Result<(), StoreError> {
    validate_auth(auth)?;
    validate_credential_reference(credential_reference)?;
    let supported = match name {
        "github" => matches!(auth, IntegrationAuth::Bearer { .. }),
        "searxng" => matches!(
            auth,
            IntegrationAuth::None | IntegrationAuth::Bearer { .. } | IntegrationAuth::ApiKey { .. }
        ),
        "opensearch" => matches!(
            auth,
            IntegrationAuth::None | IntegrationAuth::Bearer { .. } | IntegrationAuth::Basic { .. }
        ),
        _ => false,
    };
    if !supported {
        return Err(StoreError::Adapter(
            "native integration auth type is not supported".into(),
        ));
    }
    let partial_basic = matches!(auth, IntegrationAuth::Basic { .. })
        && !credential_references.is_empty()
        && !credentials_satisfy_auth(auth, credential_reference, credential_references);
    let misplaced = match auth {
        IntegrationAuth::None => {
            credential_reference.is_some() || !credential_references.is_empty()
        }
        IntegrationAuth::Basic { .. } => credential_reference.is_some(),
        _ => !credential_references.is_empty(),
    };
    if partial_basic || misplaced {
        return Err(StoreError::Adapter(
            "native integration credential references do not match its auth type".into(),
        ));
    }
    Ok(())
}
