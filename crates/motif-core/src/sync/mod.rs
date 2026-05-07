//! Sync layer: the seam between Motif and the upstream controller.
//!
//! v0.0.2-alpha.2 reshaped this layer around the [`Controller`] trait
//! plus a worker per controller (native `std::thread`, wasm
//! `wasm-bindgen-futures`). The engine calls into [`MutationLog`] on
//! commit; the log forwards via a channel sender to the worker; the
//! worker calls `Controller::apply` on the other side. See
//! `MOTIF.md` decisions 3, 12, 18.

mod controller;
mod mutation;
mod mutation_log;
mod worker;

pub use controller::Controller;
#[cfg(feature = "in-memory-controller")]
pub use controller::{InMemoryController, InMemoryHandle};
pub use mutation::{ActorId, Mutation, MutationOp};
pub use mutation_log::MutationLog;
pub use worker::{spawn_controller_worker, WorkerHandle};
