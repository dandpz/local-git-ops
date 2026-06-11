//! Library surface of `local-git-ops`, exposed so integration tests (and
//! potential embedders) can drive the analysis pipeline directly.

pub mod cli;
pub mod export;
pub mod filter;
pub mod history;
pub mod loc;
pub mod metrics;
pub mod render;
pub mod sanitize;
