//! Authenticated browser and outbound runtime transports for the cloud application.
mod auth;
mod certificates;
/// Secret-reference-only Kubernetes and local host configuration.
pub mod config;
mod connection;
mod http;
/// Composed HTTP/OIDC and mutual-TLS server lifecycle.
pub mod server;
mod tls;
