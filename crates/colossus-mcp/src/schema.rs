//! MCP schemas are self-contained declarations, not additional I/O authority.

use jsonschema::{Retrieve, Uri, ValidationError, Validator};
use serde_json::Value;

struct NoExternalSchemas;

impl Retrieve for NoExternalSchemas {
    fn retrieve(
        &self,
        _uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(std::io::Error::other("external MCP schema references are unsupported").into())
    }
}

pub(super) fn validator(schema: &Value) -> Result<Validator, Box<ValidationError<'static>>> {
    jsonschema::options()
        .with_retriever(NoExternalSchemas)
        .build(schema)
        .map_err(Box::new)
}
