//! Narrow integration with credentials managed by the official Codex CLI.

#![allow(clippy::missing_errors_doc)]

mod cli;
mod confined_store;
mod store;

pub use cli::*;
pub use store::*;

/// Credential reference reserved for a file-backed Codex sign-in.
pub const CODEX_CREDENTIAL_REFERENCE: &str = "codex:default";
/// ChatGPT Codex backend used by the official Codex client.
pub const CODEX_API_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// Official Codex wire-contract version audited by the Colossus adapter.
///
/// This is intentionally independent of the Colossus package version. Update it only
/// after reviewing the corresponding official Codex request and response contracts.
/// It also negotiates the model catalog, so stale versions can hide newer models.
/// Audited against `openai/codex` tag `rust-v0.163.0`, commit
/// `83086188ae0614bd882818ff1aa23f3194962012`: `model-provider-info` sets the
/// `version` header, `models-manager` uses whole-semver `client_version`, and
/// `codex-api` retains slug-based model cards and POST responses with SSE output.
pub const CODEX_PROTOCOL_VERSION: &str = "0.163.0";
/// OAuth token endpoint used to refresh a Codex sign-in.
pub const CODEX_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
/// Exact origin required in a permit when refreshing a Codex sign-in.
pub const CODEX_AUTH_ORIGIN: &str = "https://auth.openai.com";
