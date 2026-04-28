//! `motif-core` is the engine library for Motif, a tiny embedded follower
//! graph store. v0.0.1 alpha.2 ships only the configuration layer and the
//! sync skeleton — no storage, no query engine yet. See `MOTIF.md` at the
//! repo root for the milestone plan.

pub mod config;
pub mod sync;

pub use config::{
    ConfigError, ControllerConfig, ControllerKind, IdentityConfig, MotifConfig, StorageConfig,
};
pub use sync::{
    ActorId, ControllerClient, InMemoryControllerClient, Mutation, MutationKind, MutationLog,
};
