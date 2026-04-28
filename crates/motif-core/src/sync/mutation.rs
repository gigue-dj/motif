//! A controller-bound mutation record. v0.0.1 keeps `wal_payload` opaque —
//! we forward serialized engine bytes verbatim. v0.0.2 will replace this
//! with a structured diff once the SurrealQL boundary is defined.

use serde::{Deserialize, Serialize};

/// Identity of the actor who produced a mutation. Per-user + per-device
/// because v0.0.1 anticipates compromised / shared devices: the controller
/// must distinguish "this user from device A" from "same user from device
/// B" for audit and conflict resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorId {
    pub user_id: String,
    pub device_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MutationKind {
    NodeInsert,
    NodeUpdate,
    NodeDelete,
    RelInsert,
    RelUpdate,
    RelDelete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mutation {
    /// Monotonically increasing local sequence number, assigned by
    /// `MutationLog::record`. The controller uses this to detect dropped
    /// or reordered mutations on the wire.
    pub local_seq: u64,
    pub kind: MutationKind,
    pub actor: ActorId,
    pub table_name: String,
    pub wal_payload: Vec<u8>,
}
