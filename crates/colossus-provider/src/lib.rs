//! Permit-bound model-provider adapters and normalized provider events.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
pub use colossus_codex_auth::{
    CODEX_API_BASE_URL, CODEX_AUTH_ORIGIN, CODEX_CREDENTIAL_REFERENCE, CodexAuthStore,
};
use colossus_codex_auth::{
    CODEX_PROTOCOL_VERSION, CODEX_TOKEN_ENDPOINT, CodexAuthError, CodexAuthorization,
    CodexRefreshRequest,
};
use colossus_contracts::{
    CredentialReference, EffectRequest, ModelCapabilities, ModelContent, ModelContentPart,
    ModelImageReference, ModelLimits, ModelMessage, ModelMessageRole, ModelRequest, ModelRoute,
    ModelToolCall, ModelToolDefinition, ProviderEvent, ProviderModelInfo, ProviderReadiness,
    ProviderReadinessCheck, ProviderResponseDiagnostic, ProviderStreamItem, ProviderTurn,
    ProviderUsage, QuarantinedEffectResult, ReasoningEffort, ResourceAuthority,
    validate_model_message_content, validate_model_transcript,
};
use colossus_network::AdditionalRootCertificates;
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, NetworkDestinationMatch,
    QuarantinedEffectObserver, StreamingEffectExecutor, http_transport_authority_match,
    non_public_network_address,
};
use colossus_ports::{CredentialResolutionError, RunInputMediaResolver};
use futures::StreamExt as _;
use reqwest::{Client, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::net::lookup_host;

const MAX_PROVIDER_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_PROVIDER_REQUEST_WITH_IMAGES_BYTES: usize = 44 * 1024 * 1024;
const MAX_PROVIDER_ADDRESSES: usize = 16;
const MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES: usize = 16 * 1024;
const MAX_CODEX_REFRESH_RESPONSE_BYTES: usize = 256 * 1024;

impl From<CredentialResolutionError> for ProviderError {
    fn from(error: CredentialResolutionError) -> Self {
        Self::Credential(error.to_string())
    }
}

mod normalization;
use normalization::*;

mod tool_names;
use tool_names::*;

mod streaming;
use streaming::*;

mod profile;
pub use profile::*;

mod executor;
pub use executor::*;

mod registry;
pub use registry::*;

#[cfg(test)]
mod tests;
