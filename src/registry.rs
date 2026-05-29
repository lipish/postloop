use chrono::Utc;
use std::path::{Path, PathBuf};

use crate::storage::{Storage, StreamWriter};

/// Persistent metadata for one recorded agent session.
///
/// Stored under `sessions/{id}` key in memmap_fs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionSummary {
    /// Unique session identifier (UUID v7).
    pub id: String,
    /// The original command line that was executed.
    pub agent_cmd: String,
    /// Working directory at session start.
    pub cwd: String,
    /// "running" | "succeeded" | "failed" | "interrupted"
    pub status: String,
    pub start_at: String,
    pub end_at: Option<String>,
    pub exit_code: Option<i64>,
    /// Reference to the primary stdout stream (usually a memmap_fs key).
    pub log_path: String,
    pub thought_count: i64,
}

/// A single thought / reasoning event extracted from the agent session.
///
/// Stored as JSONL under `sessions/{id}/thoughts`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ThoughtEvent {
    pub seq: i64,
    pub ts: String,
    pub event_type: String,
    pub content: String,
}

/// High-level application storage facade over memmap_fs.
///
/// Owns session metadata, stream append/read, and search indexing.
/// All higher-level commands (run, list, show, search...) go through a Registry.
pub struct Registry {
    storage: Storage,
}

impl Registry {
    pub fn init(repo_root: &Path) -> Result<Self, anyhow::Error> {
        let storage_root = resolve_storage_root(repo_root);

        // Initialize memmap_fs storage
        let storage = Storage::init(&storage_root)?;

        Ok(Self { storage })
    }

    pub fn create_session(
        &self,
        session_id: &str,
        agent_cmd: &str,
        cwd: &Path,
        log_ref: &str,
    ) -> Result<(), anyhow::Error> {
        let meta = SessionSummary {
            id: session_id.to_string(),
            agent_cmd: agent_cmd.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            status: "running".to_string(),
            start_at: Utc::now().to_rfc3339(),
            end_at: None,
            exit_code: None,
            log_path: log_ref.to_string(),
            thought_count: 0,
        };

        // Store in memmap_fs
        self.storage.put_session(&meta)?;
        self.storage.index_session(session_id)?;

        Ok(())
    }

    pub fn add_thought_events(
        &self,
        session_id: &str,
        event_type: &str,
        lines: &[String],
        start_seq: i64,
    ) -> Result<(i64, i64), anyhow::Error> {
        let mut seq = start_seq;
        let mut added_count = 0;
        let mut jsonl = String::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }

            let ev = ThoughtEvent {
                seq,
                ts: Utc::now().to_rfc3339(),
                event_type: event_type.to_string(),
                content: line.clone(),
            };

            let serialized = serde_json::to_string(&ev)?;
            jsonl.push_str(&serialized);
            jsonl.push('\n');
            seq += 1;
            added_count += 1;
        }

        if !jsonl.is_empty() {
            self.append_stream(session_id, "thoughts", jsonl.as_bytes())?;
        }

        Ok((seq, added_count))
    }

    pub fn set_thought_count(
        &self,
        session_id: &str,
        thought_count: i64,
    ) -> Result<(), anyhow::Error> {
        // Update in memmap_fs
        if let Some(mut meta) = self.storage.get_session(session_id)? {
            meta.thought_count = thought_count;
            self.storage.put_session(&meta)?;
        }
        Ok(())
    }

    pub fn complete_session(
        &self,
        session_id: &str,
        status: &str,
        exit_code: Option<i32>,
    ) -> Result<(), anyhow::Error> {
        // Update in memmap_fs
        if let Some(mut meta) = self.storage.get_session(session_id)? {
            meta.status = status.to_string();
            meta.end_at = Some(Utc::now().to_rfc3339());
            meta.exit_code = exit_code.map(|v| v as i64);
            self.storage.put_session(&meta)?;
        }
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionSummary>, anyhow::Error> {
        Ok(self.storage.get_session(session_id)?)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, anyhow::Error> {
        Ok(self.storage.list_sessions()?)
    }

    /// 获取最近一次会话（按 start_at 倒序）。
    ///
    /// 如果没有任何会话，返回 `Ok(None)`。
    pub fn get_latest_session(&self) -> Result<Option<SessionSummary>, anyhow::Error> {
        let mut sessions = self.list_sessions()?;
        if sessions.is_empty() {
            return Ok(None);
        }
        sessions.sort_by(|a, b| b.start_at.cmp(&a.start_at));
        Ok(sessions.into_iter().next())
    }

    pub fn append_stream(
        &self,
        session_id: &str,
        stream: &str,
        data: &[u8],
    ) -> Result<(), anyhow::Error> {
        Ok(self.storage.append_stream(session_id, stream, data)?)
    }

    pub fn stream_writer(&self, session_id: &str, stream: &str) -> StreamWriter {
        self.storage.stream_writer(session_id, stream)
    }

    pub fn read_stream_to_bytes(
        &self,
        session_id: &str,
        stream: &str,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.storage.read_stream_to_bytes(session_id, stream)?)
    }

    // ─── Full-text Search ─────────────────────────────────────────────────────

    /// Index conversation content for full-text search.
    pub fn index_conversation(
        &self,
        session_id: &str,
        conversation: &[String],
    ) -> Result<(), anyhow::Error> {
        // Index each conversation turn
        for (i, turn) in conversation.iter().enumerate() {
            let key = format!("{}/turn/{}", session_id, i);
            self.storage.index_text(&key, turn)?;
        }
        Ok(())
    }

    /// Search across all indexed conversations.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, anyhow::Error> {
        let hits = self.storage.search(query, limit)?;
        let mut results = Vec::new();

        for hit in hits {
            // Parse session_id from key (format: "session_id/turn/N")
            let parts: Vec<&str> = hit.key.split('/').collect();
            if let Some(session_id) = parts.first() {
                if let Some(session) = self.get_session(session_id)? {
                    results.push(SearchResult {
                        session_id: session_id.to_string(),
                        session,
                        score: hit.score,
                    });
                }
            }
        }

        // Deduplicate by session_id, keeping highest score
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut seen = std::collections::HashSet::new();
        results.retain(|r| seen.insert(r.session_id.clone()));

        Ok(results)
    }
}

/// Result of a full-text search across all indexed conversations.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub session_id: String,
    pub session: SessionSummary,
    pub score: f32,
}

fn resolve_storage_root(repo_root: &Path) -> PathBuf {
    // 会话数据（sessions/）默认隔离在用户全局 ~/.intentloop 下，避免污染仓库。
    // 启动配置（agents.toml / shell_setup 等）已彻底移除，不再由 IntentLoop 管理。
    // 如需 per-repo 会话存储，显式 export INTENTLOOP_HOME=/path/to/local-dir（建议加入 .gitignore）。
    if let Ok(custom_root) = std::env::var("INTENTLOOP_HOME") {
        let custom_root = custom_root.trim();
        if !custom_root.is_empty() {
            return PathBuf::from(custom_root);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let home = home.trim();
        if !home.is_empty() {
            return PathBuf::from(home).join(".intentloop");
        }
    }

    // 无 HOME 环境（如某些容器）下的最后回退：放 repo/.intent/sessions（该目录通常已在 .gitignore）
    repo_root.join(".intent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_session_persists_metadata_without_business_session_files() {
        let dir = tempdir().unwrap();
        let storage = Storage::init(dir.path()).unwrap();
        let registry = Registry { storage };

        registry
            .create_session(
                "test-session",
                "echo hello",
                Path::new("/tmp"),
                "memmap_fs:sessions/test-session/stdout",
            )
            .unwrap();

        let session = registry.get_session("test-session").unwrap().unwrap();
        assert_eq!(session.id, "test-session");
        assert_eq!(session.log_path, "memmap_fs:sessions/test-session/stdout");
        assert!(!dir.path().join("sessions").join("test-session").exists());
    }

    #[test]
    fn add_thought_events_writes_jsonl_to_memmap_stream() {
        let dir = tempdir().unwrap();
        let storage = Storage::init(dir.path()).unwrap();
        let registry = Registry { storage };
        let lines = vec!["first".to_string(), "".to_string(), "second".to_string()];

        let (next_seq, added) = registry
            .add_thought_events("test-session", "stdout", &lines, 7)
            .unwrap();

        assert_eq!(next_seq, 9);
        assert_eq!(added, 2);
        let bytes = registry
            .read_stream_to_bytes("test-session", "thoughts")
            .unwrap();
        let content = String::from_utf8(bytes).unwrap();
        assert!(content.contains("\"seq\":7"));
        assert!(content.contains("\"event_type\":\"stdout\""));
        assert!(content.contains("\"content\":\"second\""));
    }

    #[test]
    fn get_latest_session_returns_most_recent_by_start_at() {
        use std::thread;
        use std::time::Duration;

        let dir = tempdir().unwrap();
        let storage = Storage::init(dir.path()).unwrap();
        let registry = Registry { storage };

        // Create first session
        registry
            .create_session(
                "sess-1",
                "echo one",
                Path::new("/tmp"),
                "memmap_fs:sessions/sess-1/stdout",
            )
            .unwrap();

        // Ensure a measurable time gap
        thread::sleep(Duration::from_millis(2));

        // Create second (later) session
        registry
            .create_session(
                "sess-2",
                "echo two",
                Path::new("/tmp"),
                "memmap_fs:sessions/sess-2/stdout",
            )
            .unwrap();

        let latest = registry.get_latest_session().unwrap().unwrap();
        assert_eq!(latest.id, "sess-2");
        assert_eq!(latest.agent_cmd, "echo two");
    }

    #[test]
    fn get_latest_session_returns_none_when_empty() {
        let dir = tempdir().unwrap();
        let storage = Storage::init(dir.path()).unwrap();
        let registry = Registry { storage };

        let latest = registry.get_latest_session().unwrap();
        assert!(latest.is_none());
    }
}
