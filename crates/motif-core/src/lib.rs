//! `motif-core` is the engine library for Motif, a tiny embedded
//! follower graph store. v0.0.2-alpha.2 ships:
//!
//! - configuration (TOML via `serde`)
//! - graph data shapes (`Node`, `Edge`, `Value`, `Properties`)
//! - storage (`Storage` trait + `FileStorage` + `MemoryStorage`)
//! - persisted Mutation log (the on-disk record format IS the sync log)
//! - hand-rolled Cypher subset parser + interpreter, including the
//!   `_motif.X` metadata-as-data namespace
//! - a [`sync::Controller`] trait + worker-thread-per-controller
//!   scaffolding (native `std::thread`, wasm `wasm-bindgen-futures`)
//!
//! See `MOTIF.md` at the repo root for the milestone plan.

pub mod capability;
pub mod config;
pub mod engine;
pub mod graph;
pub mod query;
pub mod record;
pub mod schema;
pub mod storage;
pub mod sync;
pub mod value;

pub use capability::{probe as probe_capability, resolve as resolve_capability};
pub use config::{
    CapabilityConfig, ConfigError, ControllerConfig, EdgeConfig, IdentityConfig, MotifConfig,
    StorageConfig,
};
pub use engine::{Engine, EngineError};
pub use graph::{Edge, Node, Properties};
pub use query::{Params, QueryError, QueryResult, ResultCell, Statement};
pub use schema::{PropertyType, Schema, TableKind, TableSchema};
pub use storage::{FileStorage, MemoryStorage, Storage, StorageError};
pub use sync::{
    spawn_controller_worker, ActorId, Controller, ControllerError, Mutation, MutationLog,
    MutationOp, WorkerHandle,
};
#[cfg(feature = "in-memory-controller")]
pub use sync::{InMemoryController, InMemoryHandle};
pub use value::Value;
