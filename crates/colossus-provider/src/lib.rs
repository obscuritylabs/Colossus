//! Permit-bound model-provider adapters and normalized provider events.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_codex_auth::{
    CODEX_API_BASE_URL, CODEX_AUTH_ORIGIN, CODEX_CREDENTIAL_REFERENCE, CODEX_PROTOCOL_VERSION,
    CODEX_TOKEN_ENDPOINT, CodexAuthError, CodexAuthStore, CodexAuthorization, CodexRefreshRequest,
};
use colossus_contracts::{
    CredentialReference, EffectRequest, ModelCapabilities, ModelLimits, ModelMessage,
    ModelMessageRole, ModelRequest, ModelRoute, ModelToolCall, ModelToolDefinition, ProviderEvent,
    ProviderModelInfo, ProviderReadiness, ProviderReadinessCheck, ProviderResponseDiagnostic,
    ProviderStreamItem, ProviderTurn, ProviderUsage, QuarantinedEffectResult, ReasoningEffort,
};
use colossus_network::AdditionalRootCertificates;
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, NetworkDestinationMatch,
    QuarantinedEffectObserver, StreamingEffectExecutor, network_destination_match,
    non_public_network_address,
};
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
const MAX_PROVIDER_ADDRESSES: usize = 16;
const MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES: usize = 16 * 1024;
const MAX_CODEX_REFRESH_RESPONSE_BYTES: usize = 256 * 1024;

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
