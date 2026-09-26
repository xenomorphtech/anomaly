//! Commander core — the world model, its persistence, the wire protocol between
//! the daemon and its frontends, and (with the `host` feature) the backend
//! engine that runs wasm building programs and codex worker units.

pub mod model;
pub mod proto;
pub mod store;

#[cfg(feature = "host")]
pub mod codex;
#[cfg(feature = "host")]
pub mod engine;
#[cfg(feature = "host")]
pub mod wasm;
#[cfg(feature = "host")]
pub mod worker;
