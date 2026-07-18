use super::*;

/// Tool catalog construction failure.
#[derive(Debug, Error)]
pub enum ToolCatalogError {
    /// Duplicate, malformed, or unsupported specification.
    #[error("invalid tool catalog: {0}")]
    Invalid(String),
}

/// Immutable active tool catalog.
pub struct StaticToolRegistry {
    specs: BTreeMap<String, ToolSpec>,
}

impl StaticToolRegistry {
    /// Validate unique names, effect identities, bounds, and JSON Schemas.
    pub fn new(specs: impl IntoIterator<Item = ToolSpec>) -> Result<Self, ToolCatalogError> {
        let mut indexed = BTreeMap::new();
        for spec in specs {
            validate_spec(&spec)?;
            let name = spec.name.clone();
            if indexed.insert(name.clone(), spec).is_some() {
                return Err(ToolCatalogError::Invalid(format!(
                    "duplicate tool name {name}"
                )));
            }
        }
        Ok(Self { specs: indexed })
    }

    /// Construct the supported built-in subset by exact name.
    pub fn builtins(names: &[String]) -> Result<Self, ToolCatalogError> {
        let requested = names.iter().cloned().collect::<BTreeSet<_>>();
        if requested.len() != names.len() {
            return Err(ToolCatalogError::Invalid(
                "configured tool names must be unique".into(),
            ));
        }
        let known = builtin_specs()
            .into_iter()
            .map(|spec| (spec.name.clone(), spec))
            .collect::<BTreeMap<_, _>>();
        let mut selected = Vec::new();
        for name in requested {
            selected.push(
                known
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| ToolCatalogError::Invalid(format!("unknown tool {name}")))?,
            );
        }
        Self::new(selected)
    }
}

impl ToolRegistry for StaticToolRegistry {
    fn list_specs(&self) -> Vec<ToolSpec> {
        self.specs.values().cloned().collect()
    }

    fn validate(&self, call: &ToolCall) -> Result<ToolSpec, ToolError> {
        let spec = self
            .specs
            .get(&call.name)
            .ok_or_else(|| ToolError::Unknown(call.name.clone()))?;
        if !call.arguments.is_object() {
            return Err(ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: "arguments must be a JSON object".into(),
            });
        }
        let validator = jsonschema::validator_for(&spec.input_schema).map_err(|error| {
            ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: format!("registered schema is invalid: {error}"),
            }
        })?;
        let errors = validator
            .iter_errors(&call.arguments)
            .take(8)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            return Err(ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: errors.join("; "),
            });
        }
        Ok(spec.clone())
    }
}

/// Return every supported built-in tool name in deterministic order.
pub fn builtin_names() -> Vec<String> {
    let mut names = builtin_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Convert application specs to provider-neutral definitions.
pub fn model_definitions(registry: &dyn ToolRegistry) -> Vec<ModelToolDefinition> {
    registry
        .list_specs()
        .into_iter()
        .map(|spec| ModelToolDefinition {
            name: spec.name,
            description: spec.description,
            input_schema: spec.input_schema,
        })
        .collect()
}

fn validate_spec(spec: &ToolSpec) -> Result<(), ToolCatalogError> {
    if spec.name.is_empty()
        || spec.description.is_empty()
        || spec.max_output_bytes == 0
        || !spec.input_schema.is_object()
    {
        return Err(ToolCatalogError::Invalid(
            "tool name, description, object schema, and output bound are required".into(),
        ));
    }
    match (&spec.effect_action, &spec.capability) {
        (Some(action), Some(capability)) if !action.is_empty() && !capability.is_empty() => {}
        (None, None) => {}
        _ => {
            return Err(ToolCatalogError::Invalid(format!(
                "tool {} must configure both effect action and capability or neither",
                spec.name
            )));
        }
    }
    jsonschema::validator_for(&spec.input_schema)
        .map_err(|error| ToolCatalogError::Invalid(format!("{}: {error}", spec.name)))?;
    Ok(())
}
