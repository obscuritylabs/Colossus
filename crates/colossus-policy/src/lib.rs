//! Non-bypassable effect gateway, built-in policy, and OPA adapter.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, ActorType, ApprovalProof, DecisionOutcome, EffectPhase, EffectRequest,
    EventClassification, NewEvent, PolicyDecision, PolicyObligations, QuarantinedEffectResult,
    RiskLevel, RiskRecommendation, RiskStatus,
};
use colossus_ports::{
    ApprovalProvider, EventJournal, PolicyDecisionPoint, PolicyError, RiskEvaluator, StoreError,
};
use hmac::{Hmac, Mac};
use reqwest::{Certificate, Client, Identity, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::IpAddr,
    path::Path,
    sync::{
        Arc, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

mod kernel;
pub use kernel::*;
use kernel::{
    HmacSha256, PERMIT_LIFETIME_MS, PermitClaims, approval_proof, canonical_bytes,
    disclosure_summary, now_unix_ms, sha256_hex,
};

mod gateway;
pub use gateway::*;

mod builtin;
#[cfg(test)]
use builtin::default_obligations;
pub use builtin::*;

mod approval;
pub use approval::*;

mod opa;
pub use opa::*;

mod request;
pub use request::*;

#[cfg(test)]
mod tests;
