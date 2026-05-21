use chrono::Utc;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub intent_id: String,
    pub intent_title: String,
    pub agent_cmd: String,
    pub cwd: String,
    pub status: String,
    pub start_at: String,
    pub end_at: Option<String>,
    pub exit_code: Option<i64>,
    pub log_path: String,
    pub thought_count: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ThoughtEvent {
    pub seq: i64,
    pub ts: String,
    pub event_type: String,
    pub content: String,
}

pub struct Registry {
    storage_root: PathBuf,
}

impl Registry {
    pub fn init(repo_root: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let storage_root = resolve_storage_root(repo_root);
        let sessions_dir = storage_root.join("sessions");
        fs::create_dir_all(&sessions_dir)?;

        Ok(Self {
            storage_root,
        })
    }

    pub fn create_session(
        &self,
        session_id: &str,
        intent_id: &str,
        intent_title: &str,
        agent_cmd: &str,
        cwd: &Path,
        log_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let meta = SessionSummary {
            id: session_id.to_string(),
            intent_id: intent_id.to_string(),
            intent_title: intent_title.to_string(),
            agent_cmd: agent_cmd.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            status: "running".to_string(),
            start_at: Utc::now().to_rfc3339(),
            end_at: None,
            exit_code: None,
            log_path: log_path.to_string_lossy().to_string(),
            thought_count: 0,
        };

        let session_dir = self.session_dir_path(session_id);
        fs::create_dir_all(&session_dir)?;

        let meta_path = session_dir.join("meta.json");
        let content = serde_json::to_string_pretty(&meta)?;
        fs::write(meta_path, content)?;
        Ok(())
    }

    pub fn add_thought_events(
        &self,
        session_id: &str,
        event_type: &str,
        lines: &[String],
        start_seq: i64,
    ) -> Result<(i64, i64), Box<dyn std::error::Error>> {
        let session_dir = self.session_dir_path(session_id);
        let events_path = session_dir.join("thought_events.jsonl");

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&events_path)?;

        let mut seq = start_seq;
        let mut added_count = 0;
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
            writeln!(file, "{}", serialized)?;
            seq += 1;
            added_count += 1;
        }

        Ok((seq, added_count))
    }

    pub fn set_thought_count(
        &self,
        session_id: &str,
        thought_count: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let meta_path = self.session_dir_path(session_id).join("meta.json");
        if !meta_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&meta_path)?;
        let mut meta: SessionSummary = serde_json::from_str(&content)?;
        meta.thought_count = thought_count;
        let new_content = serde_json::to_string_pretty(&meta)?;
        fs::write(&meta_path, new_content)?;
        Ok(())
    }

    pub fn complete_session(
        &self,
        session_id: &str,
        status: &str,
        exit_code: Option<i32>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let meta_path = self.session_dir_path(session_id).join("meta.json");
        if meta_path.exists() {
            let content = fs::read_to_string(&meta_path)?;
            let mut meta: SessionSummary = serde_json::from_str(&content)?;
            meta.status = status.to_string();
            meta.end_at = Some(Utc::now().to_rfc3339());
            meta.exit_code = exit_code.map(|v| v as i64);
            let new_content = serde_json::to_string_pretty(&meta)?;
            fs::write(&meta_path, new_content)?;
        }
        Ok(())
    }

    pub fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionSummary>, Box<dyn std::error::Error>> {
        let meta_path = self.session_dir_path(session_id).join("meta.json");
        if !meta_path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&meta_path)?;
        let meta: SessionSummary = serde_json::from_str(&content)?;
        Ok(Some(meta))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, Box<dyn std::error::Error>> {
        let sessions_dir = self.storage_root.join("sessions");
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let meta_path = path.join("meta.json");
                if meta_path.exists() {
                    if let Ok(content) = fs::read_to_string(&meta_path) {
                        if let Ok(meta) = serde_json::from_str::<SessionSummary>(&content) {
                            sessions.push(meta);
                        }
                    }
                }
            }
        }

        // Sort by start_at descending
        sessions.sort_by(|a, b| b.start_at.cmp(&a.start_at));
        Ok(sessions)
    }

    pub fn session_log_path(&self, session_id: &str) -> PathBuf {
        self.session_dir_path(session_id).join("terminal.raw.log")
    }

    pub fn session_report_path(&self, session_id: &str) -> PathBuf {
        self.session_dir_path(session_id).join("report.md")
    }

    pub fn session_dir_path(&self, session_id: &str) -> PathBuf {
        self.storage_root.join("sessions").join(session_id)
    }
}

fn resolve_storage_root(repo_root: &Path) -> PathBuf {
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

    repo_root.join(".intent")
}
