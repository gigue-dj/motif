//! On-disk record format.
//!
//! Each record on disk is laid out as:
//!
//! ```text
//! [u32 LE: payload_len]  [payload_len bytes: bincode(Record)]
//! ```
//!
//! There is intentionally no checksum in v0.0.1: a torn write surfaces as
//! a `bincode` decode error during recovery, and the engine truncates the
//! tail at the first such error. CRC + crash-safety semantics land
//! alongside the persistent `MutationLog` in v0.0.2.
//!
//! The file is preceded by a 16-byte header — see `storage.rs`.

use serde::{Deserialize, Serialize};

use crate::graph::{Edge, Node};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Record {
    NodeInsert(Node),
    EdgeInsert(Edge),
}

impl Record {
    pub fn id(&self) -> &str {
        match self {
            Record::NodeInsert(n) => &n.id,
            Record::EdgeInsert(e) => &e.id,
        }
    }
}

/// Length-prefix framing constant.
pub(crate) const LEN_PREFIX_BYTES: usize = 4;

/// `bincode 2` config used for all on-disk encoding. `standard()` is the
/// little-endian, fixed-int, no-limit configuration. Pinning it explicitly
/// here so that any future bincode default change does not silently break
/// on-disk compatibility.
fn codec_config() -> bincode::config::Configuration {
    bincode::config::standard()
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("record payload length {0} exceeds u32 max")]
    LengthOverflow(usize),
    #[error("encode failed: {0}")]
    Encode(#[from] bincode::error::EncodeError),
    #[error("decode failed: {0}")]
    Decode(#[from] bincode::error::DecodeError),
}

/// Encode a record into a length-prefixed byte vector ready for append.
pub fn encode_framed(record: &Record) -> Result<Vec<u8>, RecordError> {
    let payload = bincode::serde::encode_to_vec(record, codec_config())?;
    let len_u32 =
        u32::try_from(payload.len()).map_err(|_| RecordError::LengthOverflow(payload.len()))?;

    let mut out = Vec::with_capacity(LEN_PREFIX_BYTES + payload.len());
    out.extend_from_slice(&len_u32.to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Decode a single record from `bytes`, returning the record and the
/// number of bytes consumed (always `LEN_PREFIX_BYTES + payload_len`).
/// Returns `Ok(None)` on a clean EOF (zero bytes).
///
/// A short read in the middle of a record is reported as
/// `RecordError::Decode` so the caller can truncate the tail.
pub fn decode_framed(bytes: &[u8]) -> Result<Option<(Record, usize)>, RecordError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() < LEN_PREFIX_BYTES {
        return Err(RecordError::Decode(
            bincode::error::DecodeError::UnexpectedEnd {
                additional: LEN_PREFIX_BYTES - bytes.len(),
            },
        ));
    }
    let mut len_buf = [0u8; LEN_PREFIX_BYTES];
    len_buf.copy_from_slice(&bytes[..LEN_PREFIX_BYTES]);
    let payload_len = u32::from_le_bytes(len_buf) as usize;

    let total = LEN_PREFIX_BYTES + payload_len;
    if bytes.len() < total {
        return Err(RecordError::Decode(
            bincode::error::DecodeError::UnexpectedEnd {
                additional: total - bytes.len(),
            },
        ));
    }

    let payload = &bytes[LEN_PREFIX_BYTES..total];
    let (record, _consumed_payload) =
        bincode::serde::decode_from_slice::<Record, _>(payload, codec_config())?;
    Ok(Some((record, total)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, Node};
    use crate::value::Value;

    #[test]
    fn round_trips_node() {
        let n = Node::new("u_1", "Person")
            .with_property("name", "Alice")
            .with_property("age", Value::I64(30));
        let framed = encode_framed(&Record::NodeInsert(n.clone())).unwrap();
        let (decoded, consumed) = decode_framed(&framed).unwrap().unwrap();
        assert_eq!(consumed, framed.len());
        match decoded {
            Record::NodeInsert(got) => assert_eq!(got, n),
            other => panic!("unexpected record kind: {other:?}"),
        }
    }

    #[test]
    fn round_trips_edge() {
        let e = Edge::new("e_1", "FOLLOWS", "u_1", "u_2");
        let framed = encode_framed(&Record::EdgeInsert(e.clone())).unwrap();
        let (decoded, _) = decode_framed(&framed).unwrap().unwrap();
        match decoded {
            Record::EdgeInsert(got) => assert_eq!(got, e),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn empty_input_returns_none() {
        assert!(decode_framed(&[]).unwrap().is_none());
    }

    #[test]
    fn truncated_length_prefix_is_decode_error() {
        let err = decode_framed(&[1, 2]).unwrap_err();
        assert!(matches!(err, RecordError::Decode(_)));
    }

    #[test]
    fn truncated_payload_is_decode_error() {
        let n = Node::new("u_1", "Person");
        let mut framed = encode_framed(&Record::NodeInsert(n)).unwrap();
        framed.truncate(framed.len() - 1);
        let err = decode_framed(&framed).unwrap_err();
        assert!(matches!(err, RecordError::Decode(_)));
    }
}
