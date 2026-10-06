//! Authenticated browser and outbound runtime transports for the cloud application.
mod admin;
mod auth;
mod certificates;
/// Canonical server command and its compatibility entry point.
pub mod cli;
/// Secret-reference-only Kubernetes and local host configuration.
pub mod config;
mod connection;
mod http;
/// Explicit offline import from the former protected cloud journal.
pub mod migration;
mod observability;
/// Composed HTTP/OIDC and mutual-TLS server lifecycle.
pub mod server;
mod settings;
mod tls;
