//! A controller-bound mutation record. v0.0.2-alpha.1 collapses the old
//! `Record` enum and the old `MutationKind` enum into a single shape:
//! every committed graph operation is a [`Mutation`] whose payload is a
//! [`MutationOp`]. This is also what gets persisted on disk — see
//! `record.rs` for the framing codec.

use serde::{Deserialize, Serialize};

use crate::graph::{Edge, Node};

/// Identity of the actor who produced a mutation. Per-user + per-device
/// because v0.0.1+ anticipates compromised / shared devices: the
/// controller must distinguish "this user from device A" from "same user
/// from device B" for audit and conflict resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorId {
    pub user_id: String,
    pub device_id: String,
}

/// What the mutation actually does to the graph. Replaces the v0.0.1
/// `Record` enum and `MutationKind` enum (closing PR #1 audit findings 1
/// and 2: the Rel/Edge naming inconsistency goes away, and there is no
/// more asymmetric `wal_payload` since the operation carries its own
/// data structurally).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MutationOp {
    NodeInsert(Node),
    EdgeInsert(Edge),
    NodeDelete(String),
    EdgeDelete(String),
}

impl MutationOp {
    /// The id this operation targets. Used by the in-memory index and
    /// by the foreshadow tracker.
    pub fn target_id(&self) -> &str {
        match self {
            MutationOp::NodeInsert(n) => &n.id,
            MutationOp::EdgeInsert(e) => &e.id,
            MutationOp::NodeDelete(id) | MutationOp::EdgeDelete(id) => id,
        }
    }

    /// Convenience: is this an insert (vs. a delete)?
    pub fn is_insert(&self) -> bool {
        matches!(self, MutationOp::NodeInsert(_) | MutationOp::EdgeInsert(_))
    }

    /// Convenience: does this target a node (vs. an edge)?
    pub fn is_node(&self) -> bool {
        matches!(self, MutationOp::NodeInsert(_) | MutationOp::NodeDelete(_))
    }
}

/// A committed mutation. The on-disk log is a sequence of these — the
/// persisted MutationLog and the storage layer are the same structure
/// from v0.0.2-alpha.1 onward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mutation {
    /// Monotonic local sequence number, assigned by [`MutationLog::record`].
    /// The controller uses this to detect dropped or reordered mutations
    /// on the wire and to address the mutation when issuing confirms /
    /// overrides.
    pub local_seq: u64,
    /// Who produced the mutation.
    pub actor: ActorId,
    /// `true` until the controller has confirmed (or overridden) the
    /// mutation. Local readers see foreshadow data the same as confirmed
    /// data unless they explicitly query the `_motif` metadata
    /// namespace (`MATCH (n) WHERE n._motif.foreshadow = true RETURN n`).
    /// On a fresh write with no controller wired this stays `true`
    /// forever — alpha.2 wires the confirmation flow.
    pub foreshadow: bool,
    /// The graph operation.
    pub op: MutationOp,
}
