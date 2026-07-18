//! Permit-bound filesystem, process-sandbox, and network adapters.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    EffectRequest, FilesystemGrant, PolicyObligations, QuarantinedEffectResult,
};
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, MIN_OCI_EFFECT_TIMEOUT_MS,
    MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS, MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS,
};
use command_group::CommandGroup as _;
use futures::{StreamExt as _, stream::FuturesUnordered};
use globset::{Glob, GlobMatcher};
use hmac::{Hmac, Mac};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use reqwest::{Client, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use sysinfo::{Pid as SystemPid, ProcessRefreshKind, ProcessesToUpdate, System};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, lookup_host},
    process::Command as TokioCommand,
    sync::{Semaphore, oneshot},
};
use uuid::Uuid;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use nono::{AccessMode, CapabilitySet, Sandbox};

#[cfg(target_os = "windows")]
use colossus_windows_process::{ResourceLimitViolation, SpawnRequest as WindowsSpawnRequest};
#[cfg(target_os = "windows")]
use rappct::{
    AppContainerProfile, AppContainerSid,
    acl::{self, AccessMask, ResourcePath},
    net::LoopbackExemptionGuard,
};

mod common;
use common::*;
pub use common::{ProcessSpec, SandboxDoctorReport, SandboxExecutorConfig, sandbox_doctor};

mod filesystem;
pub use filesystem::FilesystemExecutor;
use filesystem::*;

mod process;
pub use process::SandboxProcessExecutor;
use process::*;

mod helper;
use helper::*;
pub use helper::{SandboxHelperError, run_helper_stdio};

mod oci;
use oci::*;

mod supervisor;
use supervisor::*;

mod http;
use http::*;
pub use http::{HttpExecutor, run_oci_proxy_from_environment};

#[cfg(test)]
mod tests;
