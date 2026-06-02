// unwrap/expect are forbidden in production code but fine in unit tests.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod app_state;
pub mod bootstrap;
pub mod cli;
pub mod config;
pub mod db;
pub mod http;
pub mod ingest;
pub mod jobs;
pub mod livelog;
pub mod metrics_layer;
pub mod notify;
pub mod security;
pub mod sentry;
