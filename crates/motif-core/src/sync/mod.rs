//! Sync layer: the seam between Motif and the upstream controller.
//! Ported 1:1 from the C++ skeleton at `cpp-reference/src/sync/`.

mod controller_client;
mod mutation;
mod mutation_log;

pub use controller_client::{ControllerClient, InMemoryControllerClient};
pub use mutation::{ActorId, Mutation, MutationKind};
pub use mutation_log::MutationLog;
