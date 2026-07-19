//! Event-sourced integration connections, OpenAPI compilation, and permit-bound HTTP execution.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, CredentialReference, EffectRequest, EventClassification, ExecutionContext,
    IntegrationAuth, IntegrationConnection, IntegrationKind, IntegrationOperation,
    IntegrationStatus, IntegrationSummary, NewEvent, PackInstallation, PackStatus, PublisherTrust,
    QuarantinedEffectResult, ToolSpec,
};
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, NetworkDestinationMatch,
    network_destination_match, non_public_network_address,
};
use colossus_ports::{AggregateRepository, EventJournal, ExtensionRepository, StoreError};
use futures::StreamExt as _;
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::net::lookup_host;
use url::Url;

const MAX_CONNECTIONS: usize = 1_000;
const MAX_OPERATIONS: usize = 256;
const MAX_SCHEMA_BYTES: usize = 1024 * 1024;
const MAX_DESCRIPTION_BYTES: usize = 8 * 1024;

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn execution(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

mod schema;
use schema::*;

mod native_http;
use native_http::*;

mod repository;
pub use repository::*;

mod executor;
#[cfg(test)]
use executor::redact_exact_secret;
pub use executor::*;

mod openapi;
pub use openapi::compile_openapi;

mod native;
pub use native::compile_native;

#[cfg(test)]
mod tests;
