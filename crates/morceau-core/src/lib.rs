//! `morceau-core` is the engine library for Morceau, a tiny embedded
//! follower graph store. The on-disk record format is the persisted
//! mutation log; each store is a single file with a 16-byte header.
//! Single writer; the engine takes `&mut self` for both reads and
//! writes.
//!
//! ## What's in this crate
//!
//! - Configuration loaded from `morceau.toml` ([`MorceauConfig`],
//!   [`config`]).
//! - Graph data shapes ([`Node`], [`Edge`], [`Value`], [`Properties`]).
//! - Storage layer: [`Storage`] trait + [`FileStorage`] (native) +
//!   [`MemoryStorage`] (tests / wasm fallback). Wasm hosts plug their
//!   own backend via `morceau-wasm`'s `WasmHostStorage`.
//! - [`Engine`] — the public engine surface: `open` / `open_in_memory`
//!   / `open_with`, `insert_node` / `insert_edge`, `get_node` /
//!   `get_edge`, `iter_edges_by_label` / `iter_edges_from` /
//!   `iter_edges_incident_to`, `delete_node` / `delete_edge` /
//!   `delete_node_with_cascade`, `query`, `with_controller` /
//!   `with_named_controller` / `with_controller_spawned_by`,
//!   `apply_schema` / `current_schema`, `replay_unconfirmed`.
//! - [`Schema`] / [`TableSchema`] / [`PropertyType`] — controller-
//!   pushed schema with label + property-type validation.
//! - Cypher subset: `CREATE`, `MATCH (a)-[r:LABEL {prop: v}]->(b)`,
//!   multi-pattern + multi-hop, `WHERE`, `RETURN n.prop`,
//!   `ORDER BY`, `LIMIT`, `count(n)`, `collect(n.prop)`, `MERGE`,
//!   `DELETE` / `DETACH DELETE`. Plus the `_morceau.X` metadata-as-
//!   data namespace (per `MORCEAU.md` decision 19).
//! - [`sync::Controller`] trait + per-controller worker
//!   ([`spawn_controller_worker`], native [`Spawner`] trait,
//!   `Engine::with_controller`). [`Mutation`], [`MutationLog`].
//! - [`probe_capability`] / [`resolve_capability`] — deterministic
//!   capability profile for the host, native + wasm probes.
//!
//! ## Pre-v0.1 status
//!
//! All crates ship `publish = true` at v0.0.4 with a "fluid API;
//! expect breakage" stance. The public surface is stable enough to
//! commit to crates.io but not frozen — `v0.1.0` is when the API
//! freezes with semver promises (see `MORCEAU.md` long-run strategy).
//!
//! See `MORCEAU.md` for the milestone plan and `LIMITATIONS.md` for
//! the running ledger of known caveats.

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
    CapabilityConfig, ConfigError, ControllerConfig, EdgeConfig, IdentityConfig, MorceauConfig,
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
#[cfg(not(target_arch = "wasm32"))]
pub use sync::{spawn_controller_worker_with, Spawner, StdThreadSpawner};
#[cfg(feature = "in-memory-controller")]
pub use sync::{InMemoryController, InMemoryHandle};
pub use value::Value;
