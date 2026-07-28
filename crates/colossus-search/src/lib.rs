//! Permit-bound provider-neutral web-search adapters and role routing.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    CredentialReference, EffectRequest, QuarantinedEffectResult, SearchProfileSummary,
    SearchRequest, SearchResponse, SearchResult,
};
use colossus_network::AdditionalRootCertificates;
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, NetworkDestinationMatch,
    network_destination_match, non_public_network_address,
};
use futures::StreamExt as _;
use reqwest::{Client, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::net::lookup_host;

const MAX_QUERY_BYTES: usize = 4_096;
const MAX_RESULTS: usize = 20;
const DEFAULT_RESULTS: usize = 10;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SEARCH_ADDRESSES: usize = 16;
const MAX_TITLE_CHARS: usize = 4_096;
const MAX_URL_CHARS: usize = 8_192;
const MAX_SNIPPET_CHARS: usize = 32_768;
const MAX_SOURCE_CHARS: usize = 256;

mod normalization;
pub use normalization::default_search_limit;
use normalization::{
    normalize_endpoint, normalize_response, redact_exact_secret, resolve_search_addresses,
    search_execution_error, url_host_is_non_public_literal, valid_credential_reference,
    valid_header_name, valid_name, validate_credential_disclosure, validate_request,
};

mod profile;
pub use profile::*;

mod executor;
pub use executor::*;

mod registry;
pub use registry::*;

#[cfg(test)]
mod tests;
