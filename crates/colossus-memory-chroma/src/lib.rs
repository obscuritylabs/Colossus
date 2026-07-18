//! Permit-bound semantic-memory adapters for Chroma and embedding profiles.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, CredentialReference, EffectRequest, QuarantinedEffectResult,
};
use colossus_policy::{
    EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit, GatewayError, effect_request,
};
use colossus_ports::{EmbeddingProvider, MemoryIndex, StoreError};
use futures::StreamExt as _;
use reqwest::{Client, Method, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::lookup_host;

mod common;
use common::*;

mod transport;
use transport::*;

mod embeddings;
pub use embeddings::*;

mod chroma;
use chroma::ChromaOperation;
pub use chroma::*;

mod index;
pub use index::*;
#[cfg(test)]
use index::{ProjectionState, persist_position, read_position};

#[cfg(test)]
mod tests;
