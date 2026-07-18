//! Configured Model Context Protocol adapters executed through Colossus permits.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, CredentialReference, EffectRequest, ExecutionContext, FilesystemGrant,
    QuarantinedEffectResult,
};
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit, effect_request};
use colossus_sandbox::{ProcessSpec, SandboxProcessExecutor};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, Implementation,
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

mod executor;
use executor::resolve_path;
pub use executor::*;
#[cfg(test)]
use executor::{protocol_input, redact_value};

mod config;
pub use config::*;
use config::{
    INITIALIZE_REQUEST_ID, MAX_PROTOCOL_LINE_BYTES, MCP_REQUEST_ID, McpEffectInput,
    environment_reference, validate_name,
};

#[cfg(test)]
mod tests;
