//! `motif-core` is the engine library for Motif, a tiny embedded follower
//! graph store. v0.0.1 alpha.3 ships configuration, the sync skeleton,
//! and a minimal append-only storage engine (`Engine`) with node and
//! edge insert + get-by-id. Query layer and `MutationLog` wiring follow
//! in alphas 4-5. See `MOTIF.md` at the repo root for the milestone plan.

pub mod config;
pub mod engine;
pub mod graph;
pub mod record;
pub mod storage;
pub mod sync;
pub mod value;

pub use config::{
    ConfigError, ControllerConfig, ControllerKind, IdentityConfig, MotifConfig, StorageConfig,
};
pub use engine::{Engine, EngineError};
pub use graph::{Edge, Node, Properties};
pub use storage::{FileStorage, MemoryStorage, Storage, StorageError};
pub use sync::{
    ActorId, ControllerClient, InMemoryControllerClient, Mutation, MutationKind, MutationLog,
};
pub use value::Value;
