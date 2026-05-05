//! `motif-core` is the engine library for Motif, a tiny embedded follower
//! graph store. v0.0.1 alpha.4 ships configuration, the sync skeleton,
//! a single-file append-only storage engine, and a hand-rolled parser +
//! interpreter for a tiny Cypher subset (`CREATE`, `MATCH`/`WHERE`/
//! `RETURN`/`LIMIT`, `MERGE`, `MATCH ... DELETE`). `MutationLog` wiring
//! and the wasm-bindgen API land in alpha.5. See `MOTIF.md` at the repo
//! root for the milestone plan.

pub mod config;
pub mod engine;
pub mod graph;
pub mod query;
pub mod record;
pub mod storage;
pub mod sync;
pub mod value;

pub use config::{
    ConfigError, ControllerConfig, ControllerKind, IdentityConfig, MotifConfig, StorageConfig,
};
pub use engine::{Engine, EngineError};
pub use graph::{Edge, Node, Properties};
pub use query::{Params, QueryError, QueryResult, ResultCell, Statement};
pub use storage::{FileStorage, MemoryStorage, Storage, StorageError};
pub use sync::{
    ActorId, ControllerClient, InMemoryControllerClient, Mutation, MutationKind, MutationLog,
};
pub use value::Value;
