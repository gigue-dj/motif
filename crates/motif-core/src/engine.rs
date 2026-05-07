//! Storage engine: orchestrates [`Storage`] backends with the on-disk
//! Mutation log and an in-memory `id → offset` index.
//!
//! v0.0.2-alpha.1 surface:
//!
//! - `Engine::open(&MotifConfig)` — file-backed, replays the log on open.
//! - `Engine::open_with(&MotifConfig, Box<dyn Storage>)` — pluggable.
//! - `Engine::insert_node` / `insert_edge` — durable append; produces a
//!   foreshadow=true [`Mutation`].
//! - `Engine::delete_node` / `delete_edge` — durable append; produces a
//!   foreshadow=true [`Mutation`].
//! - `Engine::get_node` / `get_edge` — by user-provided string id.
//! - `Engine::is_foreshadow(id)` — true while the latest mutation
//!   targeting `id` has not yet been controller-confirmed.
//! - `Engine::query(cypher, &Params)` — runs a Cypher-subset query;
//!   `_motif.foreshadow` and other metadata-as-data namespaces are
//!   resolved at projection time.
//!
//! The on-disk log IS the persisted MutationLog: each on-disk record is
//! a bincoded [`Mutation`]. Single writer; the engine takes `&mut self`
//! for both mutation and read operations because the underlying
//! [`Storage`] seeks the file cursor on `read_at`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::{CapabilityConfig, EdgeConfig, MotifConfig};
use crate::graph::{Edge, Node};
use crate::query::{self, Params, QueryError, QueryResult};
use crate::record::{decode_framed, encode_framed, RecordError, LEN_PREFIX_BYTES};
use crate::schema::Schema;
use crate::storage::{FileStorage, MemoryStorage, Storage, StorageError, HEADER_LEN};
use crate::sync::{ActorId, Mutation, MutationLog, MutationOp};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("record codec error: {0}")]
    Record(#[from] RecordError),
    #[error("duplicate id: {0}")]
    DuplicateId(String),
    #[error("missing referenced node: {0}")]
    MissingNode(String),
    #[error("recovery failed at offset {offset}: {source}")]
    Recovery {
        offset: u64,
        #[source]
        source: RecordError,
    },
    #[error("query error: {0}")]
    Query(#[from] QueryError),
    #[error(
        "unknown label {label}: not declared in the current schema (version {schema_version})"
    )]
    SchemaUnknown { label: String, schema_version: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Node,
    Edge,
}

#[derive(Debug, Clone, Copy)]
struct IndexEntry {
    /// Byte offset of the framed Mutation in the underlying storage.
    offset: u64,
    /// Length of the bincoded Mutation payload (excluding the 4-byte
    /// length prefix).
    payload_len: u32,
    /// Whether this id targets a node or an edge. Disambiguates the
    /// id-shared map.
    kind: Kind,
    /// Whether the latest mutation that produced this index entry is
    /// still foreshadow=true. Updated on every commit; flipped by the
    /// (alpha.2) controller-confirm flow.
    foreshadow: bool,
}

pub struct Engine {
    storage: Box<dyn Storage>,
    /// Maps user-provided id → on-disk record location + foreshadow.
    /// Globally unique id space across nodes and edges.
    index: HashMap<String, IndexEntry>,
    /// Per-instance actor identity copied from `MotifConfig::identity`.
    /// Stamped onto every [`Mutation`] produced by this engine.
    actor: ActorId,
    /// Monotonic local sequence number. Starts at 1; bumped before each
    /// commit. Recovery rewinds it to one past the highest seen
    /// `local_seq` so subsequent commits stay strictly increasing.
    next_local_seq: u64,
    /// In-memory queue of foreshadowed mutations awaiting controller
    /// confirmation. Wired by `with_controller`; alpha.4 will let the
    /// Controller worker drain it.
    mutation_log: Option<Arc<MutationLog>>,
    /// The latest controller-pushed schema applied to this engine.
    /// `None` until a host or bridge calls [`Engine::apply_schema`].
    /// When `None`, label validation is permissive (any label is
    /// accepted); when `Some`, mutations against unknown labels surface
    /// [`EngineError::SchemaUnknown`].
    current_schema: Option<Schema>,
    /// Per-instance capability profile copied from
    /// `MotifConfig::capability`. Forwarded to the controller worker
    /// at `Engine::with_controller` time so bridges can route based
    /// on host facts (RAM, cores, etc.). v0.0.2-alpha.4 stores it;
    /// v0.0.3+ may also expose it via the `_motif.capability.X`
    /// metadata namespace once auto-discovery lands.
    capability: CapabilityConfig,
    /// Per-instance edge-strategy config copied from `MotifConfig::edge`.
    /// Drives the controller worker's retry/backoff in alpha.4; other
    /// fields are stored for future alphas.
    edge_config: EdgeConfig,
}

impl Engine {
    /// Open the engine using the path from `config.storage`.
    ///
    /// On `wasm32-unknown-unknown` this will fail at the first file system
    /// call — use [`Engine::open_in_memory`] there until alpha.4 wires a
    /// host-provided storage backend.
    pub fn open(config: &MotifConfig) -> Result<Self, EngineError> {
        let storage = FileStorage::open(&config.storage.path)?;
        Self::open_with(config, Box::new(storage))
    }

    /// Open the engine in memory. The `path` field of `config.storage` is
    /// ignored. Intended for tests and for the wasm path.
    pub fn open_in_memory(config: &MotifConfig) -> Result<Self, EngineError> {
        Self::open_with(config, Box::new(MemoryStorage::new()))
    }

    /// Open with a caller-provided backend.
    pub fn open_with(config: &MotifConfig, storage: Box<dyn Storage>) -> Result<Self, EngineError> {
        let mut engine = Self {
            storage,
            index: HashMap::new(),
            actor: ActorId {
                user_id: config.identity.user_id.clone(),
                device_id: config.identity.device_id.clone(),
            },
            next_local_seq: 1,
            mutation_log: None,
            current_schema: None,
            capability: config.capability.clone(),
            edge_config: config.edge.clone(),
        };
        engine.recover()?;
        Ok(engine)
    }

    /// Wire an in-memory [`MutationLog`] for callers that want to
    /// install their own forwarder (rare — most callers should use
    /// [`Engine::with_controller`] instead, which handles MutationLog
    /// + worker spawning together).
    pub fn with_mutation_log(mut self, log: Arc<MutationLog>) -> Self {
        self.mutation_log = Some(log);
        self
    }

    /// Wire a [`Controller`]. The engine creates an internal
    /// [`MutationLog`] and spawns a worker that drains it into the
    /// controller (one thread on native, one
    /// `wasm-bindgen-futures::spawn_local` task on wasm). Subsequent
    /// commits enqueue mutations onto the worker's channel and return
    /// without waiting for `Controller::apply`.
    ///
    /// The returned engine retains a handle to its MutationLog (for
    /// `mutation_count` diagnostics on the wasm path); the worker
    /// itself is detached. Drop the engine to drop the channel sender,
    /// which causes the worker to flush and exit.
    pub fn with_controller<C: crate::sync::Controller>(mut self, controller: C) -> Self {
        let log = Arc::new(MutationLog::new());
        let _handle = crate::sync::spawn_controller_worker(
            &log,
            controller,
            self.capability.clone(),
            self.edge_config.clone(),
        );
        self.mutation_log = Some(log);
        self
    }

    /// Read-only access to the host's reported capability profile.
    /// `None` of the inner `Option<>` fields just means the host
    /// didn't declare that fact in the TOML; auto-discovery is v0.0.3+.
    pub fn capability(&self) -> &CapabilityConfig {
        &self.capability
    }

    /// Read-only access to the engine's edge-strategy config.
    pub fn edge_config(&self) -> &EdgeConfig {
        &self.edge_config
    }

    /// Test/inspection helper: read the current actor identity.
    pub fn actor(&self) -> &ActorId {
        &self.actor
    }

    /// Test/inspection helper: how many mutations are buffered in the
    /// MutationLog (i.e. recorded but not yet forwarded to a worker).
    /// Returns 0 if no MutationLog is wired.
    pub fn buffered_mutation_count(&self) -> usize {
        self.mutation_log
            .as_ref()
            .map(|l| l.buffered_len())
            .unwrap_or(0)
    }

    /// Replay the log from the start, rebuilding the in-memory index
    /// and the foreshadow map. On a torn-write decode error at the tail,
    /// truncate the file back to the last good record and continue. On
    /// a decode error in the middle of the log, surface
    /// `EngineError::Recovery`.
    fn recover(&mut self) -> Result<(), EngineError> {
        let total = self.storage.len();
        let mut cursor: u64 = HEADER_LEN;
        let mut last_good: u64 = HEADER_LEN;

        while cursor < total {
            let remaining = (total - cursor) as usize;
            if remaining < LEN_PREFIX_BYTES {
                break;
            }
            let len_bytes = self.storage.read_at(cursor, LEN_PREFIX_BYTES)?;
            let payload_len =
                u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]);
            let total_record = LEN_PREFIX_BYTES + payload_len as usize;
            if remaining < total_record {
                self.storage.truncate(last_good)?;
                return Ok(());
            }

            let frame = self.storage.read_at(cursor, total_record)?;
            match decode_framed(&frame) {
                Ok(Some((mutation, consumed))) => {
                    debug_assert_eq!(consumed, total_record);
                    self.apply_recovered(&mutation, cursor, payload_len);
                    if mutation.local_seq >= self.next_local_seq {
                        self.next_local_seq = mutation.local_seq + 1;
                    }
                    cursor += total_record as u64;
                    last_good = cursor;
                }
                Ok(None) => break,
                Err(source) => {
                    return Err(EngineError::Recovery {
                        offset: cursor,
                        source,
                    });
                }
            }
        }

        if cursor < total {
            self.storage.truncate(last_good)?;
        }
        Ok(())
    }

    /// Apply a Mutation we just decoded during recovery. Inserts /
    /// deletes touch the id index; `SchemaApply` updates the engine's
    /// current schema. Each path is idempotent under replay.
    fn apply_recovered(&mut self, m: &Mutation, offset: u64, payload_len: u32) {
        match &m.op {
            MutationOp::NodeInsert(n) => {
                self.index.insert(
                    n.id.clone(),
                    IndexEntry {
                        offset,
                        payload_len,
                        kind: Kind::Node,
                        foreshadow: m.foreshadow,
                    },
                );
            }
            MutationOp::EdgeInsert(e) => {
                self.index.insert(
                    e.id.clone(),
                    IndexEntry {
                        offset,
                        payload_len,
                        kind: Kind::Edge,
                        foreshadow: m.foreshadow,
                    },
                );
            }
            MutationOp::NodeDelete(id) | MutationOp::EdgeDelete(id) => {
                self.index.remove(id);
            }
            MutationOp::SchemaApply(s) => {
                self.current_schema = Some(s.clone());
            }
        }
    }

    pub fn insert_node(&mut self, node: Node) -> Result<(), EngineError> {
        if self.index.contains_key(&node.id) {
            return Err(EngineError::DuplicateId(node.id));
        }
        self.validate_label(&node.label)?;
        self.commit(MutationOp::NodeInsert(node))
    }

    pub fn insert_edge(&mut self, edge: Edge) -> Result<(), EngineError> {
        if self.index.contains_key(&edge.id) {
            return Err(EngineError::DuplicateId(edge.id));
        }
        if !self.has_node(&edge.from) {
            return Err(EngineError::MissingNode(edge.from));
        }
        if !self.has_node(&edge.to) {
            return Err(EngineError::MissingNode(edge.to));
        }
        self.validate_label(&edge.label)?;
        self.commit(MutationOp::EdgeInsert(edge))
    }

    /// Apply a controller-pushed schema. The schema is persisted via a
    /// `MutationOp::SchemaApply` record on the on-disk log, so it
    /// survives reopen, and is teed to the controller worker like any
    /// other commit (most controllers ignore their own schema echoes
    /// via the default `Controller::flush` no-op).
    ///
    /// v0.0.2-alpha.3 takes the whole schema atomically: a later
    /// version supersedes the earlier one in its entirety. Incremental
    /// schema migrations land in v0.0.3+.
    pub fn apply_schema(&mut self, schema: Schema) -> Result<(), EngineError> {
        // Persist + update in-memory state in one commit.
        self.commit(MutationOp::SchemaApply(schema))
    }

    /// Read-only access to the latest applied schema. `None` if no
    /// schema has been pushed; callers should treat that as
    /// "permissive — accept any label".
    pub fn current_schema(&self) -> Option<&Schema> {
        self.current_schema.as_ref()
    }

    /// Reject mutations with labels that the current schema doesn't
    /// know. Permissive when no schema is set (the controller hasn't
    /// pushed one yet, or is operating without one — both valid in
    /// v0.0.2-alpha.3).
    fn validate_label(&self, label: &str) -> Result<(), EngineError> {
        let Some(schema) = &self.current_schema else {
            return Ok(());
        };
        if schema.has_label(label) {
            Ok(())
        } else {
            Err(EngineError::SchemaUnknown {
                label: label.to_owned(),
                schema_version: schema.version,
            })
        }
    }

    /// Delete a node by id. v0.0.2 still does not enforce referential
    /// integrity: edges that reference a deleted node become dangling
    /// (the query layer treats them as unreachable). `DETACH DELETE`-
    /// style cascade is a v0.0.3+ design item.
    pub fn delete_node(&mut self, id: &str) -> Result<bool, EngineError> {
        if !self.has_node(id) {
            return Ok(false);
        }
        self.commit(MutationOp::NodeDelete(id.to_owned()))?;
        Ok(true)
    }

    pub fn delete_edge(&mut self, id: &str) -> Result<bool, EngineError> {
        if !self.has_edge(id) {
            return Ok(false);
        }
        self.commit(MutationOp::EdgeDelete(id.to_owned()))?;
        Ok(true)
    }

    /// Build a foreshadow=true Mutation, frame it, append to storage,
    /// publish to the in-memory index, and tee to the in-memory
    /// MutationLog (if wired). Single source of truth for "what just
    /// landed on disk."
    fn commit(&mut self, op: MutationOp) -> Result<(), EngineError> {
        let local_seq = self.next_local_seq;
        self.next_local_seq += 1;

        let m = Mutation {
            local_seq,
            actor: self.actor.clone(),
            foreshadow: true,
            op,
        };

        let frame = encode_framed(&m)?;
        let payload_len = (frame.len() - LEN_PREFIX_BYTES) as u32;
        let offset = self.storage.append(&frame)?;

        // Update in-memory state from the same Mutation we just
        // persisted, so on-disk state and in-memory state agree.
        match &m.op {
            MutationOp::NodeInsert(n) => {
                self.index.insert(
                    n.id.clone(),
                    IndexEntry {
                        offset,
                        payload_len,
                        kind: Kind::Node,
                        foreshadow: m.foreshadow,
                    },
                );
            }
            MutationOp::EdgeInsert(e) => {
                self.index.insert(
                    e.id.clone(),
                    IndexEntry {
                        offset,
                        payload_len,
                        kind: Kind::Edge,
                        foreshadow: m.foreshadow,
                    },
                );
            }
            MutationOp::NodeDelete(id) | MutationOp::EdgeDelete(id) => {
                self.index.remove(id);
            }
            MutationOp::SchemaApply(s) => {
                self.current_schema = Some(s.clone());
            }
        }

        // Tee to the in-memory MutationLog for the (alpha.2) controller
        // worker. No-op when no log is wired.
        if let Some(log) = &self.mutation_log {
            log.record(m);
        }
        Ok(())
    }

    /// Materialize all live nodes. v0.0.1 / v0.0.2-alpha.1 do an O(N)
    /// scan because the only secondary index is the id map. Acceptable
    /// up to alpha-era graph sizes; a label / property index is later
    /// work.
    pub fn iter_nodes(&mut self) -> Result<Vec<Node>, EngineError> {
        let entries: Vec<(String, IndexEntry)> = self
            .index
            .iter()
            .filter(|(_, e)| e.kind == Kind::Node)
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let mut out = Vec::with_capacity(entries.len());
        for (_, entry) in entries {
            if let Some(MutationOp::NodeInsert(n)) = self.read_op_at(&entry)? {
                out.push(n);
            }
        }
        Ok(out)
    }

    pub fn iter_edges(&mut self) -> Result<Vec<Edge>, EngineError> {
        let entries: Vec<(String, IndexEntry)> = self
            .index
            .iter()
            .filter(|(_, e)| e.kind == Kind::Edge)
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let mut out = Vec::with_capacity(entries.len());
        for (_, entry) in entries {
            if let Some(MutationOp::EdgeInsert(e)) = self.read_op_at(&entry)? {
                out.push(e);
            }
        }
        Ok(out)
    }

    pub fn get_node(&mut self, id: &str) -> Result<Option<Node>, EngineError> {
        let Some(entry) = self.index.get(id).copied() else {
            return Ok(None);
        };
        if entry.kind != Kind::Node {
            return Ok(None);
        }
        Ok(self.read_op_at(&entry)?.and_then(|op| match op {
            MutationOp::NodeInsert(n) => Some(n),
            _ => None,
        }))
    }

    pub fn get_edge(&mut self, id: &str) -> Result<Option<Edge>, EngineError> {
        let Some(entry) = self.index.get(id).copied() else {
            return Ok(None);
        };
        if entry.kind != Kind::Edge {
            return Ok(None);
        }
        Ok(self.read_op_at(&entry)?.and_then(|op| match op {
            MutationOp::EdgeInsert(e) => Some(e),
            _ => None,
        }))
    }

    fn read_op_at(&mut self, entry: &IndexEntry) -> Result<Option<MutationOp>, EngineError> {
        let frame_len = LEN_PREFIX_BYTES + entry.payload_len as usize;
        let bytes = self.storage.read_at(entry.offset, frame_len)?;
        let (mutation, _) = decode_framed(&bytes)?.ok_or(EngineError::Recovery {
            offset: entry.offset,
            source: RecordError::Decode(bincode::error::DecodeError::Other(
                "indexed mutation decoded to None",
            )),
        })?;
        Ok(Some(mutation.op))
    }

    pub fn has_node(&self, id: &str) -> bool {
        matches!(self.index.get(id), Some(e) if e.kind == Kind::Node)
    }

    pub fn has_edge(&self, id: &str) -> bool {
        matches!(self.index.get(id), Some(e) if e.kind == Kind::Edge)
    }

    /// True iff the latest mutation targeting `id` is still
    /// foreshadow=true. Returns `false` for unknown ids.
    ///
    /// Exposed via the `_motif.foreshadow` Cypher metadata namespace
    /// per MOTIF.md decision 19 (metadata-as-data).
    pub fn is_foreshadow(&self, id: &str) -> bool {
        self.index.get(id).map(|e| e.foreshadow).unwrap_or(false)
    }

    pub fn node_count(&self) -> usize {
        self.index.values().filter(|e| e.kind == Kind::Node).count()
    }

    pub fn edge_count(&self) -> usize {
        self.index.values().filter(|e| e.kind == Kind::Edge).count()
    }

    /// Execute a Cypher-subset query. See `query` module docs for the
    /// supported grammar.
    pub fn query(&mut self, cypher: &str, params: &Params) -> Result<QueryResult, EngineError> {
        let stmt = query::parse(cypher)?;
        let result = query::execute(self, &stmt, params).map_err(QueryError::from)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{ControllerConfig, IdentityConfig, StorageConfig};
    use crate::value::Value;

    fn cfg() -> MotifConfig {
        MotifConfig {
            identity: IdentityConfig {
                user_id: "u".into(),
                device_id: "d".into(),
            },
            controller: ControllerConfig {
                kind: "in-memory".into(),
            },
            storage: StorageConfig {
                path: PathBuf::from(":memory:"),
            },
            capability: Default::default(),
            edge: Default::default(),
        }
    }

    #[test]
    fn inserts_and_reads_back_a_node() {
        let mut e = Engine::open_in_memory(&cfg()).unwrap();
        e.insert_node(Node::new("n1", "Person").with_property("name", "Alice"))
            .unwrap();
        let got = e.get_node("n1").unwrap().unwrap();
        assert_eq!(got.id, "n1");
        assert_eq!(got.label, "Person");
        assert_eq!(got.properties["name"], Value::String("Alice".into()));
    }

    #[test]
    fn fresh_inserts_are_foreshadowed() {
        let mut e = Engine::open_in_memory(&cfg()).unwrap();
        e.insert_node(Node::new("n1", "Person")).unwrap();
        assert!(e.is_foreshadow("n1"));
        assert!(!e.is_foreshadow("nope"));
    }

    #[test]
    fn rejects_duplicate_node_id() {
        let mut e = Engine::open_in_memory(&cfg()).unwrap();
        e.insert_node(Node::new("n1", "Person")).unwrap();
        let err = e.insert_node(Node::new("n1", "Person")).unwrap_err();
        assert!(matches!(err, EngineError::DuplicateId(_)));
    }

    #[test]
    fn rejects_edge_with_missing_endpoints() {
        let mut e = Engine::open_in_memory(&cfg()).unwrap();
        let err = e
            .insert_edge(Edge::new("e1", "FOLLOWS", "missing_a", "missing_b"))
            .unwrap_err();
        assert!(matches!(err, EngineError::MissingNode(_)));
    }

    #[test]
    fn inserts_and_reads_back_an_edge() {
        let mut e = Engine::open_in_memory(&cfg()).unwrap();
        e.insert_node(Node::new("a", "Person")).unwrap();
        e.insert_node(Node::new("b", "Person")).unwrap();
        e.insert_edge(
            Edge::new("e1", "FOLLOWS", "a", "b").with_property("since", Value::I64(2026)),
        )
        .unwrap();

        let got = e.get_edge("e1").unwrap().unwrap();
        assert_eq!(got.from, "a");
        assert_eq!(got.to, "b");
        assert_eq!(got.properties["since"], Value::I64(2026));
        assert_eq!(e.node_count(), 2);
        assert_eq!(e.edge_count(), 1);
    }

    #[test]
    fn get_returns_none_for_missing_or_wrong_kind() {
        let mut e = Engine::open_in_memory(&cfg()).unwrap();
        e.insert_node(Node::new("a", "Person")).unwrap();
        assert!(e.get_node("missing").unwrap().is_none());
        assert!(e.get_edge("a").unwrap().is_none());
    }

    #[test]
    fn local_seq_is_monotonic_per_commit() {
        let mut e = Engine::open_in_memory(&cfg()).unwrap();
        assert_eq!(e.next_local_seq, 1);
        e.insert_node(Node::new("a", "Person")).unwrap();
        assert_eq!(e.next_local_seq, 2);
        e.insert_node(Node::new("b", "Person")).unwrap();
        assert_eq!(e.next_local_seq, 3);
        e.delete_node("a").unwrap();
        assert_eq!(e.next_local_seq, 4);
    }
}
