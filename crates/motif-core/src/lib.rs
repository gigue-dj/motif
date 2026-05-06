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

pub mod config;
pub mod engine;
pub mod graph;
pub mod query;
pub mod record;
pub mod storage;
pub mod sync;
pub mod value;

pub use config::{ConfigError, ControllerConfig, IdentityConfig, MotifConfig, StorageConfig};
pub use engine::{Engine, EngineError};
pub use graph::{Edge, Node, Properties};
pub use query::{Params, QueryError, QueryResult, ResultCell, Statement};
pub use storage::{FileStorage, MemoryStorage, Storage, StorageError};
pub use sync::{
    spawn_controller_worker, ActorId, Controller, InMemoryController, InMemoryHandle, Mutation,
    MutationLog, MutationOp, WorkerHandle,
};
pub use value::Value;
