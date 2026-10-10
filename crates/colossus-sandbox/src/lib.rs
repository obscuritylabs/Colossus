//! Permit-bound filesystem, process-sandbox, and network adapters.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    EffectRequest, FilesystemGrant, PolicyObligations, QuarantinedEffectResult, ResourceAuthority,
    SandboxBoundaryMode,
};
use colossus_network::AdditionalRootCertificates;
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, MIN_OCI_EFFECT_TIMEOUT_MS,
    MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS, MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS, NetworkDestinationMatch,
    QuarantinedEffectObserver, StreamingEffectExecutor, http_transport_authority_match,
    network_destination_match, non_public_network_address,
};
use command_group::CommandGroup as _;
use futures::{StreamExt as _, stream::FuturesUnordered};
use globset::{Glob, GlobMatcher};
use hmac::{Hmac, Mac};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    net::{TcpListener, TcpStream},
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
pub use common::{
    ProcessSpec, ProcessStdinCompletion, SandboxDoctorReport, SandboxExecutorConfig, sandbox_doctor,
};

mod filesystem;
pub use filesystem::FilesystemExecutor;
use filesystem::*;

mod protected_filesystem;
pub use protected_filesystem::ProtectedFilesystem;
use protected_filesystem::ProtectedFilesystemSnapshot;

mod process;
pub use process::SandboxProcessExecutor;
use process::*;

mod helper;
use helper::*;
pub use helper::{SandboxHelperError, run_helper_stdio, run_native_protection_probe};

mod oci;
use oci::*;

mod stdin_completion;
use stdin_completion::*;

mod process_stream;
pub use process_stream::ProcessControl;

mod helper_stream;
use helper_stream::*;

mod supervisor;
use supervisor::*;

mod http;
pub use http::{DEFAULT_HTTP_MAX_REDIRECTS, HttpExecutor, MAX_HTTP_REDIRECTS};

mod http_proxy;
pub use http_proxy::run_oci_proxy_from_environment;
use http_proxy::*;

mod proxy_tls;
use proxy_tls::*;

mod browser_egress;
#[cfg(target_os = "linux")]
mod browser_profiles;
#[cfg(target_os = "linux")]
pub use browser_profiles::{
    BrowserProfileEngine, BrowserProfileError, BrowserProfileLease, BrowserProfileStore,
};
#[cfg(windows)]
mod browser_windows;
pub use browser_egress::{
    BrowserEgressError, BrowserEgressLease, BrowserEgressLimits, BrowserEgressState,
};
#[cfg(windows)]
pub use browser_windows::{
    WindowsBrowserConfig, WindowsBrowserPresentation, WindowsBrowserShutdownReceipt,
    WindowsBrowserSupervisor,
};

#[cfg(target_os = "linux")]
mod browser_oci;
#[cfg(target_os = "linux")]
pub use browser_oci::{
    OciBrowserConfig, OciBrowserIdentity, OciBrowserLimits, OciBrowserPki,
    OciBrowserPkiAuthorization, OciBrowserPkiEnrollmentProvider, OciBrowserPkiRegistration,
    OciBrowserPkiRegistry, OciBrowserPkiScopeAuthorization, OciBrowserShutdownReceipt,
    OciBrowserSupervisor, OciClientIdentityBinding,
};

#[cfg(test)]
mod tests;
