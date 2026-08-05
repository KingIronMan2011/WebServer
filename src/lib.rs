//! Reusable Webserver components for the binary and fuzz targets.

pub mod admin;
pub mod app;
pub mod cli;
pub mod config;
pub mod error;
pub mod handlers;
pub mod http;
pub mod observability;
pub mod routing;
pub mod server;
pub mod tls;
pub mod upstream;

#[cfg(windows)]
pub mod windows_service_runtime;
