//! Compile only the two bundled schemas, once per process. No payload selects a schema.

use super::*;
use std::sync::OnceLock;

struct Validators {
    plugin: jsonschema::Validator,
    mcp_server: jsonschema::Validator,
}

fn validators() -> Result<&'static Validators, StoreError> {
    static VALIDATORS: OnceLock<Result<Validators, String>> = OnceLock::new();
    VALIDATORS
        .get_or_init(|| compile().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(adapter)
}

fn compile() -> Result<Validators, StoreError> {
    let plugin: Value = serde_json::from_str(PLUGIN_SCHEMA).map_err(adapter)?;
    let mcp: Value = serde_json::from_str(MCP_SCHEMA).map_err(adapter)?;
    let definitions = mcp
        .get("$defs")
        .ok_or_else(|| StoreError::Adapter("bundled MCP schema is incomplete".into()))?;
    let mcp_server = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "#/$defs/server",
        "$defs": definitions,
    });
    Ok(Validators {
        plugin: jsonschema::validator_for(&plugin).map_err(adapter)?,
        mcp_server: jsonschema::validator_for(&mcp_server).map_err(adapter)?,
    })
}

pub(crate) fn plugin_validator() -> Result<&'static jsonschema::Validator, StoreError> {
    Ok(&validators()?.plugin)
}

pub(crate) fn mcp_server_validator() -> Result<&'static jsonschema::Validator, StoreError> {
    Ok(&validators()?.mcp_server)
}
