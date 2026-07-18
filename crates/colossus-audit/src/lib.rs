//! Durable, policy-bound export of redacted audit evidence.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, ActorType, AuditEvidence, CredentialReference, EventEnvelope, ExecutionContext,
    ExternalWorkRetryState,
};
use colossus_policy::{EffectExecutor, EffectGateway, GatewayError, effect_request};
use colossus_ports::{AuditExporter, EventJournal, ExternalWorkQueue, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use url::Url;

mod common;
pub use common::*;

mod directory;
pub use directory::*;

mod worm;
pub use worm::*;

mod service;
pub use service::*;

#[cfg(test)]
mod tests;
