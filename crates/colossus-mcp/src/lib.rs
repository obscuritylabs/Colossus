//! Configured Model Context Protocol adapters executed through Colossus permits.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, CredentialReference, EffectRequest, ExecutionContext, FilesystemGrant,
    QuarantinedEffectResult, ResourceAuthority,
};
use colossus_network::AdditionalRootCertificates;
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit, effect_request};
use colossus_sandbox::{ProcessSpec, ProcessStdinCompletion};
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
use executor::{
    RemoteOperationResult, execute_remote_operation, parse_tools_result, protocol_input,
    redact_value, remote_call_failure, remote_timeout_error, tools_page_contains_secret,
};

mod config;
pub use config::*;
use config::{
    INITIALIZE_REQUEST_ID, MAX_PROTOCOL_LINE_BYTES, MCP_REQUEST_ID, McpEffectInput, validate_name,
};

mod http_client;
use http_client::HardenedStreamableHttpClient;
#[cfg(test)]
use http_client::content_type_matches;

mod oauth_store;
use oauth_store::OAuthStoreFactory;
#[cfg(test)]
use oauth_store::{OAUTH_RECORDS, OAuthCredentialStore};

mod oauth_http;
use oauth_http::HardenedOAuthHttpClient;

#[cfg(test)]
mod tests;
