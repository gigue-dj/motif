//! Sync layer: the seam between Motif and the upstream controller.
//! v0.0.2-alpha.1 collapses the old separate `Record` and `MutationKind`
//! types into a unified [`Mutation`] / [`MutationOp`] shape.

mod controller_client;
mod mutation;
mod mutation_log;

pub use controller_client::{ControllerClient, InMemoryControllerClient};
pub use mutation::{ActorId, Mutation, MutationOp};
pub use mutation_log::MutationLog;
