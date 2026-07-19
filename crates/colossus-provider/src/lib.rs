//! Permit-bound model-provider adapters and normalized provider events.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    CredentialReference, EffectRequest, ModelMessage, ModelMessageRole, ModelRequest,
    ModelToolCall, ModelToolDefinition, ProviderEvent, ProviderModelInfo, ProviderReadiness,
    ProviderReadinessCheck, ProviderStreamItem, ProviderTurn, ProviderUsage,
    QuarantinedEffectResult,
};
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
use tokio::net::lookup_host;

const MAX_PROVIDER_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_PROVIDER_ADDRESSES: usize = 16;

mod normalization;
use normalization::*;

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
