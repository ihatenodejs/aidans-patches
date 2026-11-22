//! Native APK Lab host library.
//!
//! The CLI binary is a thin formatting boundary. Domain modules return typed
//! values or typed errors, and side effects (subprocesses, filesystem writes)
//! stay in the module that owns the invariant.
//!
//! The migration sanitizer tests build a complete benchmark-result fixture with
//! nested `serde_json::json!` literals; expansion depth needs more than the
//! default.
#![recursion_limit = "256"]

pub mod archives;
pub mod cli;
pub mod error;
pub mod inspection;
pub mod migration;
pub mod models;
pub mod process;
pub mod tools;
pub mod workspace;
