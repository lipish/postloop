use chrono::Utc;
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub intent_id: String,
    pub intent_title: String,
    pub agent_cmd: String,
    pub status: String,
    pub start_at: String,
    pub end_at: Option<String>,
    pub exit_code: Option<i64>,
    pub log_path: String,
    pub thought_count: i64,
}

pub struct Registry {
    db_path: PathBuf,
    storage_root: PathBuf,
}

impl Registry {
    pub fn init(repo_root: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let storage_root = resolve_storage_root(repo_root);
        let sessions_dir = storage_root.join("sessions");
        fs::create_dir_all(&sessions_dir)?;

        let db_path = storage_root.join("db.sqlite");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;

            CREATE TABLE IF NOT EXISTS sessions (
              id TEXT PRIMARY KEY,
              intent_id TEXT NOT NULL,
              intent_title TEXT NOT NULL,
              agent_cmd TEXT NOT NULL,
              cwd TEXT NOT NULL,
              start_at TEXT NOT NULL,
              end_at TEXT,
              status TEXT NOT NULL,
              exit_code INTEGER,
              log_path TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS thought_events (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              seq INTEGER NOT NULL,
              ts TEXT NOT NULL,
              event_type TEXT NOT NULL,
              content TEXT NOT NULL,
              FOREIGN KEY(session_id) REFERENCES sessions(id)
            );
            ",
        )?;

        Ok(Self {
            db_path,
            storage_root,
        })
    }

    fn connect(&self) -> Result<Connection, Box<dyn std::error::Error>> {
        Ok(Connection::open(&self.db_path)?)
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
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO sessions (id, intent_id, intent_title, agent_cmd, cwd, start_at, status, log_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7)",
            params![
                session_id,
                intent_id,
                intent_title,
                agent_cmd,
                cwd.to_string_lossy().to_string(),
                Utc::now().to_rfc3339(),
                log_path.to_string_lossy().to_string()
            ],
        )?;
        Ok(())
    }

    pub fn add_thought_events(
        &self,
        session_id: &str,
        event_type: &str,
        lines: &[String],
        start_seq: i64,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let mut seq = start_seq;

        for line in lines {
            if line.trim().is_empty() {
                continue;
            }

            tx.execute(
                "INSERT INTO thought_events (session_id, seq, ts, event_type, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    session_id,
                    seq,
                    Utc::now().to_rfc3339(),
                    event_type,
                    line
                ],
            )?;
            seq += 1;
        }

        tx.commit()?;
        Ok(seq)
    }

    pub fn complete_session(
        &self,
        session_id: &str,
        status: &str,
        exit_code: Option<i32>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.connect()?;
        conn.execute(
            "UPDATE sessions
             SET status = ?2,
                 end_at = ?3,
                 exit_code = ?4
             WHERE id = ?1",
            params![
                session_id,
                status,
                Utc::now().to_rfc3339(),
                exit_code.map(|v| v as i64)
            ],
        )?;
        Ok(())
    }

    pub fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionSummary>, Box<dyn std::error::Error>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.intent_id, s.intent_title, s.agent_cmd, s.status, s.start_at, s.end_at, s.exit_code, s.log_path,
                    (SELECT COUNT(*) FROM thought_events t WHERE t.session_id = s.id) as thought_count
             FROM sessions s
             WHERE s.id = ?1",
        )?;

        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(SessionSummary {
                id: row.get(0)?,
                intent_id: row.get(1)?,
                intent_title: row.get(2)?,
                agent_cmd: row.get(3)?,
                status: row.get(4)?,
                start_at: row.get(5)?,
                end_at: row.get(6)?,
                exit_code: row.get(7)?,
                log_path: row.get(8)?,
                thought_count: row.get(9)?,
            }));
        }

        Ok(None)
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
