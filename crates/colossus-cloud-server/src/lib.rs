//! Authenticated browser and outbound runtime transports for the cloud application.
mod admin;
/// Separately deployed protected rollback-anchor authority for hosted Chat.
pub mod anchor_authority;
mod auth;
mod certificates;
mod chat;
pub use chat::shared::import_codex_auth;
mod chat_settings;
/// Canonical server command and its compatibility entry point.
pub mod cli;
/// Secret-reference-only Kubernetes and local host configuration.
pub mod config;
mod connection;
mod http;
mod observability;
mod resources;
/// Composed HTTP/OIDC and mutual-TLS server lifecycle.
pub mod server;
mod settings;
mod tls;
