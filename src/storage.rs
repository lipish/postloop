//! Storage layer backed by memmap_fs.
//!
//! This module provides a thin wrapper around memmap_fs to handle IntentLoop's
//! storage needs: session metadata, terminal streams, and full-text search.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use memmap_fs::MemMapFS;

use crate::registry::SessionSummary;

/// Storage handle for IntentLoop sessions.
///
/// Wraps `MemMapFS` and provides IntentLoop-specific APIs for:
/// - Session metadata CRUD
/// - Terminal stream append/read
/// - Full-text search indexing
#[derive(Clone)]
pub struct Storage {
    fs: MemMapFS,
    root: PathBuf,
}

impl Storage {
    /// Initialize storage at the given root directory.
    ///
    /// Creates the directory layout if it does not exist and replays WAL
    /// to recover any uncommitted state from a previous crash.
    pub fn init<P: Into<PathBuf>>(root: P) -> Result<Self, StorageError> {
        let root: PathBuf = root.into();
        let fs = MemMapFS::init(&root)?;
        Ok(Self { fs, root })
    }

    /// Returns the storage root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    // ─── Session Metadata ─────────────────────────────────────────────────────

    /// Store a session's metadata.
    pub fn put_session(&self, session: &SessionSummary) -> Result<(), StorageError> {
        let key = format!("sessions/{}", session.id);
        let bytes = serde_json::to_vec(session)?;
        self.fs.set_kv(key, bytes)?;
        Ok(())
    }

    /// Retrieve a session's metadata by ID.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionSummary>, StorageError> {
        let key = format!("sessions/{}", id);
        if let Some(bytes) = self.fs.get_kv(&key) {
            let session: SessionSummary = serde_json::from_slice(&bytes)?;
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    /// List all sessions, sorted by start_at descending.
    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, StorageError> {
        // memmap_fs doesn't have a native list API, so we iterate the KV store
        // This is a workaround - ideally memmap_fs would provide prefix iteration
        let mut sessions = Vec::new();

        // For now, we need to track session IDs separately
        // This is a limitation we'll address by storing a session index
        if let Some(bytes) = self.fs.get_kv("_session_index") {
            let ids: Vec<String> = serde_json::from_slice(&bytes)?;
            for id in ids {
                if let Some(session) = self.get_session(&id)? {
                    sessions.push(session);
                }
            }
        }

        // Sort by start_at descending
        sessions.sort_by(|a, b| b.start_at.cmp(&a.start_at));
        Ok(sessions)
    }

    /// Add a session ID to the index (call after put_session).
    pub fn index_session(&self, id: &str) -> Result<(), StorageError> {
        let mut ids: Vec<String> = if let Some(bytes) = self.fs.get_kv("_session_index") {
            serde_json::from_slice(&bytes)?
        } else {
            Vec::new()
        };

        if !ids.contains(&id.to_string()) {
            ids.push(id.to_string());
            let bytes = serde_json::to_vec(&ids)?;
            self.fs.set_kv("_session_index".to_string(), bytes)?;
        }
        Ok(())
    }

    // ─── Stream I/O ───────────────────────────────────────────────────────────

    /// Append data to a stream (e.g., stdout, stdin).
    pub fn append_stream(
        &self,
        session_id: &str,
        stream: &str,
        data: &[u8],
    ) -> Result<(), StorageError> {
        let key = format!("sessions/{}/{}", session_id, stream);
        self.fs.append_stream(&key, data)?;
        Ok(())
    }

    /// Open a stream for reading.
    pub fn open_stream(&self, session_id: &str, stream: &str) -> Result<impl Read, StorageError> {
        let key = format!("sessions/{}/{}", session_id, stream);
        Ok(self.fs.open_read(&key)?)
    }

    /// Read an entire stream into memory for post-processing.
    pub fn read_stream_to_bytes(
        &self,
        session_id: &str,
        stream: &str,
    ) -> Result<Vec<u8>, StorageError> {
        let mut reader = self.open_stream(session_id, stream)?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Create a `Write` sink backed by a memmap_fs stream.
    pub fn stream_writer(&self, session_id: &str, stream: &str) -> StreamWriter {
        StreamWriter {
            storage: self.clone(),
            session_id: session_id.to_string(),
            stream: stream.to_string(),
        }
    }

    // ─── Full-text Search ─────────────────────────────────────────────────────

    /// Index text for full-text search.
    pub fn index_text(&self, key: &str, text: &str) -> Result<(), StorageError> {
        self.fs.index(key, text)?;
        Ok(())
    }

    /// Search indexed text.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StorageError> {
        let hits = self.fs.search(query, limit)?;
        Ok(hits
            .into_iter()
            .map(|h| SearchHit {
                key: h.key,
                score: h.score,
            })
            .collect())
    }
}

/// A `std::io::Write` adapter that appends every write to a memmap_fs stream.
pub struct StreamWriter {
    storage: Storage,
    session_id: String,
    stream: String,
}

impl Write for StreamWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.storage
            .append_stream(&self.session_id, &self.stream, buf)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A search result hit.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub key: String,
    pub score: f32,
}

/// Storage errors.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("memmap_fs error: {0}")]
    MemMapFs(#[from] memmap_fs::error::MemMapError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
