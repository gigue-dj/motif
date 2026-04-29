//! Storage engine: orchestrates [`Storage`] backends with the on-disk
//! record format and an in-memory `id → offset` index.
//!
//! v0.0.1 alpha.3 surface:
//!
//! - `Engine::open(&MotifConfig)` — file-backed, replays the log on open
//! - `Engine::open_with(&MotifConfig, Box<dyn Storage>)` — pluggable
//! - `Engine::insert_node` / `insert_edge` — durable append
//! - `Engine::get_node` / `get_edge` — by user-provided string id
//!
//! Update, delete, transactions, and the `MutationLog` tee all land in
//! later milestones. Single writer; the engine takes `&mut self` for
//! both mutation and read operations because the underlying [`Storage`]
//! seeks the file cursor on `read_at`.

use std::collections::HashMap;

use crate::config::MotifConfig;
use crate::graph::{Edge, Node};
use crate::record::{decode_framed, encode_framed, Record, RecordError, LEN_PREFIX_BYTES};
use crate::storage::{FileStorage, MemoryStorage, Storage, StorageError, HEADER_LEN};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Node,
    Edge,
}

#[derive(Debug, Clone, Copy)]
struct IndexEntry {
    offset: u64,
    payload_len: u32,
    kind: Kind,
}

pub struct Engine {
    storage: Box<dyn Storage>,
    /// Maps user-provided id → on-disk record location.
    /// v0.0.1 keeps both nodes and edges in a single map; ids must be
    /// globally unique. v0.0.2 may split this if the namespaces collide.
    index: HashMap<String, IndexEntry>,
}

impl Engine {
    /// Open the engine using the path from `config.storage`.
    ///
    /// On `wasm32-unknown-unknown` this will fail at the first file system
    /// call — use [`Engine::open_with`] with a [`MemoryStorage`] there
    /// until alpha.5 wires a host-provided backend.
    pub fn open(config: &MotifConfig) -> Result<Self, EngineError> {
        let storage = FileStorage::open(&config.storage.path)?;
        Self::open_with(config, Box::new(storage))
    }

    /// Open the engine in memory. The `path` field of `config.storage` is
    /// ignored. Intended for tests and for the alpha.5 wasm path.
    pub fn open_in_memory(_config: &MotifConfig) -> Result<Self, EngineError> {
        Self::open_with(_config, Box::new(MemoryStorage::new()))
    }

    /// Open with a caller-provided backend.
    pub fn open_with(
        _config: &MotifConfig,
        storage: Box<dyn Storage>,
    ) -> Result<Self, EngineError> {
        let mut engine = Self {
            storage,
            index: HashMap::new(),
        };
        engine.recover()?;
        Ok(engine)
    }

    /// Replay the log from the start, rebuilding the in-memory index.
    /// On a torn-write decode error at the tail, we truncate the file
    /// back to the last good record and continue. On a decode error in
    /// the middle of the log, we surface `EngineError::Recovery`.
    fn recover(&mut self) -> Result<(), EngineError> {
        let total = self.storage.len();
        let mut cursor: u64 = HEADER_LEN;
        let mut last_good: u64 = HEADER_LEN;

        while cursor < total {
            let remaining = (total - cursor) as usize;
            // Read the length prefix; if we can't, treat it as a torn tail.
            if remaining < LEN_PREFIX_BYTES {
                break;
            }
            let len_bytes = self.storage.read_at(cursor, LEN_PREFIX_BYTES)?;
            let payload_len =
                u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]);
            let total_record = LEN_PREFIX_BYTES + payload_len as usize;
            if remaining < total_record {
                // Torn tail: truncate and stop.
                self.storage.truncate(last_good)?;
                return Ok(());
            }

            let frame = self.storage.read_at(cursor, total_record)?;
            match decode_framed(&frame) {
                Ok(Some((record, consumed))) => {
                    debug_assert_eq!(consumed, total_record);
                    let entry = IndexEntry {
                        offset: cursor,
                        payload_len,
                        kind: match &record {
                            Record::NodeInsert(_) => Kind::Node,
                            Record::EdgeInsert(_) => Kind::Edge,
                        },
                    };
                    self.index.insert(record.id().to_owned(), entry);
                    cursor += total_record as u64;
                    last_good = cursor;
                }
                Ok(None) => break,
                Err(source) => {
                    // Bad payload mid-log: this is genuinely corrupt, not
                    // a torn tail. Surface it.
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

    pub fn insert_node(&mut self, node: Node) -> Result<(), EngineError> {
        if self.index.contains_key(&node.id) {
            return Err(EngineError::DuplicateId(node.id));
        }
        let record = Record::NodeInsert(node);
        self.append_record(record, Kind::Node)
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
        let record = Record::EdgeInsert(edge);
        self.append_record(record, Kind::Edge)
    }

    fn append_record(&mut self, record: Record, kind: Kind) -> Result<(), EngineError> {
        let id = record.id().to_owned();
        let frame = encode_framed(&record)?;
        let payload_len = (frame.len() - LEN_PREFIX_BYTES) as u32;
        let offset = self.storage.append(&frame)?;
        self.index.insert(
            id,
            IndexEntry {
                offset,
                payload_len,
                kind,
            },
        );
        Ok(())
    }

    pub fn get_node(&mut self, id: &str) -> Result<Option<Node>, EngineError> {
        match self.read(id, Kind::Node)? {
            Some(Record::NodeInsert(n)) => Ok(Some(n)),
            _ => Ok(None),
        }
    }

    pub fn get_edge(&mut self, id: &str) -> Result<Option<Edge>, EngineError> {
        match self.read(id, Kind::Edge)? {
            Some(Record::EdgeInsert(e)) => Ok(Some(e)),
            _ => Ok(None),
        }
    }

    fn read(&mut self, id: &str, expected: Kind) -> Result<Option<Record>, EngineError> {
        let entry = match self.index.get(id) {
            Some(e) if e.kind == expected => *e,
            _ => return Ok(None),
        };
        let frame_len = LEN_PREFIX_BYTES + entry.payload_len as usize;
        let bytes = self.storage.read_at(entry.offset, frame_len)?;
        let (record, _) = decode_framed(&bytes)?.ok_or(EngineError::Recovery {
            offset: entry.offset,
            source: RecordError::Decode(bincode::error::DecodeError::Other(
                "indexed record decoded to None",
            )),
        })?;
        Ok(Some(record))
    }

    pub fn has_node(&self, id: &str) -> bool {
        matches!(self.index.get(id), Some(e) if e.kind == Kind::Node)
    }

    pub fn has_edge(&self, id: &str) -> bool {
        matches!(self.index.get(id), Some(e) if e.kind == Kind::Edge)
    }

    pub fn node_count(&self) -> usize {
        self.index.values().filter(|e| e.kind == Kind::Node).count()
    }

    pub fn edge_count(&self) -> usize {
        self.index.values().filter(|e| e.kind == Kind::Edge).count()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{ControllerConfig, ControllerKind, IdentityConfig, StorageConfig};
    use crate::value::Value;

    fn cfg() -> MotifConfig {
        MotifConfig {
            identity: IdentityConfig {
                user_id: "u".into(),
                device_id: "d".into(),
            },
            controller: ControllerConfig {
                kind: ControllerKind::InMemory,
            },
            storage: StorageConfig {
                path: PathBuf::from(":memory:"),
            },
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
        assert!(e.get_edge("a").unwrap().is_none()); // it's a node, not an edge
    }
}
