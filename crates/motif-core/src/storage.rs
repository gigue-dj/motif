//! Pluggable storage backend.
//!
//! v0.0.1 alpha.3 ships two implementations:
//!
//! - [`FileStorage`]: append-only file with `fsync` after each write.
//!   Native targets only; `wasm32-unknown-unknown` has no working file
//!   system, so attempting to construct one there will surface as an
//!   `io::Error` from `OpenOptions::open`.
//! - [`MemoryStorage`]: in-process `Vec<u8>`. Used by every test in this
//!   crate and intended as the default backend on `wasm32` until alpha.5
//!   wires a host-provided storage shim.
//!
//! The store opens existing files only after verifying a 16-byte header.
//! The header carries a magic prefix and a format version; v0.0.1 is
//! format `1`.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub(crate) const MAGIC: &[u8; 8] = b"MOTIF\0\0\x01";
/// On-disk format version. Bumps:
/// - 1 → 2 in v0.0.2-alpha.1: the on-disk record collapsed from
///   `Record::*` enum variants into a single `Mutation` shape.
/// - 2 → 3 in v0.0.2-alpha.3: `MutationOp` gained `SchemaApply(Schema)`.
/// - 3 → 4 in v0.0.4-alpha.3: `Value` gained `Timestamp(i64)` and
///   `List(Vec<Value>)` variants (discriminants 5 and 6); `PropertyType`
///   gained matching `Timestamp` and `List`. Existing variant tags
///   0-4 are unchanged, but a tag-5 / tag-6 value cannot be decoded
///   by older binaries — the format-version bump makes that explicit.
///
/// Older stores are rejected at open with `StorageError::BadVersion` —
/// no migration tooling per the "bleeding-edge until outside contributors
/// arrive" decision.
pub(crate) const FORMAT_VERSION: u32 = 4;
pub(crate) const HEADER_LEN: u64 = 16;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("file at {0} is not a Motif store (bad magic)")]
    BadMagic(PathBuf),
    #[error("file at {path} has unsupported format version {found} (expected {expected})")]
    BadVersion {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[error("file at {path} is shorter than the header ({len} < {expected})")]
    ShortHeader {
        path: PathBuf,
        len: u64,
        expected: u64,
    },
    #[error("truncate would corrupt the header: new_len {new_len} < HEADER_LEN {header_len}")]
    TruncateBelowHeader { new_len: u64, header_len: u64 },
    /// v0.0.3-alpha.3: a wasm host-supplied storage shim threw an
    /// exception. The message is best-effort — JsValue exceptions
    /// don't always have a useful Display impl, so the implementation
    /// falls back to "<no message>" if the host threw a non-string.
    #[error("host storage error: {message}")]
    JsHostError { message: String },
}

/// Marker trait that bounds [`Storage`] to `Send` on native targets and
/// is a no-op on `wasm32-unknown-unknown`. Wasm has no threads in the
/// base ABI, and JS-bridged types (anything holding a `JsValue`) are
/// `!Send` by design — requiring `Send` on the trait would lock out
/// the v0.0.3-alpha.3 [`crate::storage`]-shaped wasm host shim. Native
/// hosts that put an `Engine` behind an `Arc<Mutex<…>>` and ship it
/// across threads still need the `Send` bound, so we keep it where it
/// matters and drop it where it doesn't.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// Append-only single-file storage. Writes go to the end; reads are by
/// absolute byte offset (the `Engine` keeps an in-memory `id → offset`
/// index).
pub trait Storage: MaybeSend {
    /// Append `bytes` and return the offset where they began. Implementations
    /// should ensure durability before returning (e.g. `fsync`).
    fn append(&mut self, bytes: &[u8]) -> Result<u64, StorageError>;

    /// Read `len` bytes starting at `offset`.
    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, StorageError>;

    /// Total length of the store, including the header.
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() <= HEADER_LEN
    }

    /// Truncate the store to `new_len` bytes. Used by the engine during
    /// recovery to drop a torn-write tail. Implementations must persist
    /// the truncation before returning.
    ///
    /// Caller invariant: `new_len >= HEADER_LEN`. Implementations
    /// surface `StorageError::TruncateBelowHeader` if the caller would
    /// truncate into the magic / format-version header (closes PR #1
    /// review finding 6 — the engine recovery path always passes a
    /// safe value, but the guard keeps future misuse from corrupting
    /// the header).
    fn truncate(&mut self, new_len: u64) -> Result<(), StorageError>;

    /// Bytes available on the underlying medium, if the implementation
    /// can answer. Used by the v0.0.3 capability probe to populate
    /// `[capability].storage_mb` when the host hasn't declared one.
    ///
    /// `FileStorage` returns `Some(fs2::available_space(path))`.
    /// `MemoryStorage` returns `None` (no underlying medium). The
    /// wasm host-storage shim (alpha.3) defers to the host JS object.
    fn free_space(&self) -> Option<u64> {
        None
    }
}

// ---------- FileStorage ----------

pub struct FileStorage {
    path: PathBuf,
    file: File,
    len: u64,
}

impl std::fmt::Debug for FileStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileStorage")
            .field("path", &self.path)
            .field("len", &self.len)
            .finish()
    }
}

impl FileStorage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| StorageError::Io {
                path: path.clone(),
                source: e,
            })?;

        let len = file
            .metadata()
            .map_err(|e| StorageError::Io {
                path: path.clone(),
                source: e,
            })?
            .len();

        if len == 0 {
            // New file: write the header.
            let mut header = [0u8; HEADER_LEN as usize];
            header[..MAGIC.len()].copy_from_slice(MAGIC);
            header[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
            file.write_all(&header).map_err(|e| StorageError::Io {
                path: path.clone(),
                source: e,
            })?;
            file.sync_all().map_err(|e| StorageError::Io {
                path: path.clone(),
                source: e,
            })?;
            return Ok(Self {
                path,
                file,
                len: HEADER_LEN,
            });
        }

        if len < HEADER_LEN {
            return Err(StorageError::ShortHeader {
                path,
                len,
                expected: HEADER_LEN,
            });
        }

        // Existing file: validate the header.
        let mut header = [0u8; HEADER_LEN as usize];
        file.seek(SeekFrom::Start(0))
            .map_err(|e| StorageError::Io {
                path: path.clone(),
                source: e,
            })?;
        file.read_exact(&mut header).map_err(|e| StorageError::Io {
            path: path.clone(),
            source: e,
        })?;
        if &header[..MAGIC.len()] != MAGIC {
            return Err(StorageError::BadMagic(path));
        }
        let mut version_bytes = [0u8; 4];
        version_bytes.copy_from_slice(&header[MAGIC.len()..MAGIC.len() + 4]);
        let version = u32::from_le_bytes(version_bytes);
        if version != FORMAT_VERSION {
            return Err(StorageError::BadVersion {
                path,
                found: version,
                expected: FORMAT_VERSION,
            });
        }

        Ok(Self { path, file, len })
    }
}

impl Storage for FileStorage {
    fn append(&mut self, bytes: &[u8]) -> Result<u64, StorageError> {
        let offset = self.len;
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| StorageError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        self.file.write_all(bytes).map_err(|e| StorageError::Io {
            path: self.path.clone(),
            source: e,
        })?;
        self.file.sync_all().map_err(|e| StorageError::Io {
            path: self.path.clone(),
            source: e,
        })?;
        self.len += bytes.len() as u64;
        Ok(offset)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, StorageError> {
        let mut buf = vec![0u8; len];
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| StorageError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        self.file
            .read_exact(&mut buf)
            .map_err(|e| StorageError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        Ok(buf)
    }

    fn len(&self) -> u64 {
        self.len
    }

    fn truncate(&mut self, new_len: u64) -> Result<(), StorageError> {
        if new_len < HEADER_LEN {
            return Err(StorageError::TruncateBelowHeader {
                new_len,
                header_len: HEADER_LEN,
            });
        }
        self.file.set_len(new_len).map_err(|e| StorageError::Io {
            path: self.path.clone(),
            source: e,
        })?;
        self.file.sync_all().map_err(|e| StorageError::Io {
            path: self.path.clone(),
            source: e,
        })?;
        self.len = new_len;
        Ok(())
    }

    fn free_space(&self) -> Option<u64> {
        // fs2 is target-gated to non-wasm32 in Cargo.toml, but
        // FileStorage itself is buildable on wasm32 (and unusable —
        // open() fails at OpenOptions::open). Keep the module
        // compilable on wasm32 by returning None there.
        #[cfg(not(target_arch = "wasm32"))]
        {
            fs2::available_space(&self.path).ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }
}

// ---------- MemoryStorage ----------

#[derive(Default)]
pub struct MemoryStorage {
    bytes: Vec<u8>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        let mut s = Self::default();
        // Same header layout as FileStorage so recovery code can be
        // backend-agnostic.
        s.bytes.extend_from_slice(MAGIC);
        s.bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        // Pad to HEADER_LEN.
        s.bytes.resize(HEADER_LEN as usize, 0);
        s
    }
}

impl Storage for MemoryStorage {
    fn append(&mut self, bytes: &[u8]) -> Result<u64, StorageError> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(bytes);
        Ok(offset)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, StorageError> {
        let start = offset as usize;
        let end = start + len;
        if end > self.bytes.len() {
            return Err(StorageError::Io {
                path: PathBuf::from(":memory:"),
                source: io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "read past end of memory store",
                ),
            });
        }
        Ok(self.bytes[start..end].to_vec())
    }

    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn truncate(&mut self, new_len: u64) -> Result<(), StorageError> {
        if new_len < HEADER_LEN {
            return Err(StorageError::TruncateBelowHeader {
                new_len,
                header_len: HEADER_LEN,
            });
        }
        self.bytes.truncate(new_len as usize);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn memory_round_trips_append_and_read() {
        let mut s = MemoryStorage::new();
        assert_eq!(s.len(), HEADER_LEN);
        assert!(s.is_empty());

        let off = s.append(b"hello").unwrap();
        assert_eq!(off, HEADER_LEN);
        assert_eq!(s.len(), HEADER_LEN + 5);
        assert!(!s.is_empty());

        let got = s.read_at(off, 5).unwrap();
        assert_eq!(got, b"hello");
    }

    #[test]
    fn file_creates_header_on_new_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.motif");
        let s = FileStorage::open(&path).unwrap();
        assert_eq!(s.len(), HEADER_LEN);

        // Reopen and confirm the header validates.
        let s2 = FileStorage::open(&path).unwrap();
        assert_eq!(s2.len(), HEADER_LEN);
    }

    #[test]
    fn file_round_trips_across_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("b.motif");

        let off = {
            let mut s = FileStorage::open(&path).unwrap();
            s.append(b"persisted").unwrap()
        };

        let mut s = FileStorage::open(&path).unwrap();
        assert_eq!(s.len(), HEADER_LEN + 9);
        assert_eq!(s.read_at(off, 9).unwrap(), b"persisted");
    }

    #[test]
    fn file_rejects_bad_magic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.motif");
        std::fs::write(&path, [0u8; HEADER_LEN as usize]).unwrap();
        let err = FileStorage::open(&path).unwrap_err();
        assert!(matches!(err, StorageError::BadMagic(_)));
    }

    #[test]
    fn truncate_below_header_is_rejected_memory() {
        let mut s = MemoryStorage::new();
        let err = s.truncate(0).unwrap_err();
        assert!(matches!(err, StorageError::TruncateBelowHeader { .. }));
        let err = s.truncate(HEADER_LEN - 1).unwrap_err();
        assert!(matches!(err, StorageError::TruncateBelowHeader { .. }));
        // At-or-above-HEADER_LEN: ok.
        s.truncate(HEADER_LEN).unwrap();
    }

    #[test]
    fn truncate_below_header_is_rejected_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("guard.motif");
        let mut s = FileStorage::open(&path).unwrap();
        let err = s.truncate(0).unwrap_err();
        assert!(matches!(err, StorageError::TruncateBelowHeader { .. }));
        // The file's still intact; reopen succeeds.
        let _ = FileStorage::open(&path).unwrap();
    }
}
