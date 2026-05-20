use chrono::Utc;
use crate::pty::content_filter::filter_content_lines;
use crate::pty::terminal_input::{parse_submitted_lines, strip_terminal_escapes};
use crate::pty::vt100_recorder::{
    lines_added, unique_content_lines, stdout_has_ansi, ScreenSnapshot, Vt100Recorder,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurn {
    pub role: String,
    pub text: String,
    pub ts: String,
}

/// 从 PTY 原始 stdout/stdin 提取对话：stdout 走 vt100 屏幕快照，stdin 走行编辑回放。
pub fn extract_conversation(stdout: &[u8], stdin: &[u8], rows: u16, cols: u16) -> Vec<ConversationTurn> {
    let (_, turns) = extract_with_snapshots(stdout, stdin, rows, cols);
    turns
}

pub fn extract_with_snapshots(
    stdout: &[u8],
    stdin: &[u8],
    rows: u16,
    cols: u16,
) -> (Vec<ScreenSnapshot>, Vec<ConversationTurn>) {
    let ts = Utc::now().to_rfc3339();
    let user_prompts = parse_stdin_submits(stdin);
    let snapshots = Vt100Recorder::new(rows, cols).replay(stdout);
    let mut turns = Vec::new();

    if user_prompts.is_empty() {
        let text = agent_text_from_snapshots(stdout, &snapshots, 0, snapshots.len().saturating_sub(1), &[]);
        if !text.is_empty() {
            turns.push(ConversationTurn {
                role: "agent".to_string(),
                text,
                ts: ts.clone(),
            });
        }
        return (snapshots, turns);
    }

    let mut agent_start = 0usize;
    for (pi, prompt) in user_prompts.iter().enumerate() {
        turns.push(ConversationTurn {
            role: "user".to_string(),
            text: prompt.clone(),
            ts: ts.clone(),
        });

        let agent_end = if pi + 1 < user_prompts.len() {
            let next = &user_prompts[pi + 1];
            let mut j = agent_start;
            while j < snapshots.len() && !screen_contains_prompt(&snapshots[j].contents, next) {
                j += 1;
            }
            j.saturating_sub(1).max(agent_start)
        } else {
            snapshots.len().saturating_sub(1)
        };

        let prompt_str = prompt.as_str();
        let text = agent_text_from_snapshots(stdout, &snapshots, agent_start, agent_end, &[prompt_str]);
        if !text.is_empty() {
            turns.push(ConversationTurn {
                role: "agent".to_string(),
                text,
                ts: ts.clone(),
            });
        }

        agent_start = agent_end.saturating_add(1);
    }

    (snapshots, turns)
}

fn agent_text_from_snapshots(
    stdout: &[u8],
    snapshots: &[ScreenSnapshot],
    start_idx: usize,
    end_idx: usize,
    user_prompts: &[&str],
) -> String {
    let mut lines = if stdout_has_ansi(stdout) {
        lines_between_snapshots(snapshots, start_idx, end_idx)
    } else {
        unique_content_lines(&String::from_utf8_lossy(stdout))
    };
    if lines.is_empty() {
        lines = lines_between_snapshots(snapshots, start_idx, end_idx);
    }
    lines.retain(|line| !user_prompts.iter().any(|p| line.trim() == p.trim() || line.contains(p.trim())));
    filter_content_lines(lines).join("\n")
}

fn lines_between_snapshots(snapshots: &[ScreenSnapshot], start_idx: usize, end_idx: usize) -> Vec<String> {
    if snapshots.is_empty() {
        return Vec::new();
    }
    let start = start_idx.min(snapshots.len() - 1);
    let end = end_idx.min(snapshots.len() - 1);
    let mut all = Vec::new();
    let mut prev = if start > 0 {
        snapshots[start.saturating_sub(1)].contents.clone()
    } else {
        String::new()
    };
    for snap in snapshots.iter().take(end + 1).skip(start) {
        all.extend(lines_added(&prev, &snap.contents));
        prev = snap.contents.clone();
    }
    all
}

fn screen_contains_prompt(screen: &str, prompt: &str) -> bool {
    let p = prompt.trim();
    screen.contains(p) || screen.lines().any(|l| l.trim().contains(p))
}

fn parse_stdin_submits(stdin: &[u8]) -> Vec<String> {
    let cleaned = strip_terminal_escapes(stdin);
    parse_submitted_lines(&cleaned)
}

pub fn turns_to_jsonl(turns: &[ConversationTurn]) -> Vec<String> {
    turns
        .iter()
        .map(|t| {
            let escaped = t
                .text
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            format!(r#"{{"role":"{}","text":"{}","ts":"{}"}}"#, t.role, escaped, t.ts)
        })
        .collect()
}

pub fn snapshots_to_jsonl(snapshots: &[ScreenSnapshot]) -> Vec<String> {
    snapshots
        .iter()
        .map(|s| {
            let escaped = s
                .contents
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            format!(r#"{{"seq":{},"screen":"{}"}}"#, s.seq, escaped)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn split_raw_log(log: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let stdin_marker = b"[stdin]\n";
        let stdout_marker = b"[stdout]\n";
        let stderr_marker = b"[stderr]";
        let stdin_start = log
            .windows(stdin_marker.len())
            .position(|w| w == stdin_marker)
            .map(|i| i + stdin_marker.len())
            .unwrap();
        let stdout_start = log
            .windows(stdout_marker.len())
            .position(|w| w == stdout_marker)
            .map(|i| i + stdout_marker.len())
            .unwrap();
        let stdout_end = log
            .windows(stderr_marker.len())
            .position(|w| w == stderr_marker)
            .unwrap_or(log.len());
        (
            log[stdin_start..stdout_start - stdout_marker.len()].to_vec(),
            log[stdout_start..stdout_end].to_vec(),
        )
    }

    #[test]
    fn vt100_extracts_sqlite_answer() {
        let log = fs::read(
            "/Users/xinference/.intentloop/sessions/019e440e-7f24-7ce3-9c51-e0f68da0a444/terminal.raw.log",
        )
        .unwrap();
        let (stdin, stdout) = split_raw_log(&log);
        let turns = extract_conversation(&stdout, &stdin, 40, 120);
        let agent: String = turns
            .iter()
            .filter(|t| t.role == "agent")
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(agent.contains("SQLite"), "agent text: {}", agent);
        assert!(agent.contains("嵌入式"), "agent text: {}", agent);
    }

    #[test]
    fn filters_agent_tui_noise_from_real_session() {
        let log = fs::read(
            "/Users/xinference/.intentloop/sessions/019e4426-e455-7d90-a735-681b04cf6394/terminal.raw.log",
        )
        .unwrap();
        let (stdin, stdout) = split_raw_log(&log);
        let turns = extract_conversation(&stdout, &stdin, 40, 120);

        let users: Vec<_> = turns.iter().filter(|t| t.role == "user").collect();
        assert_eq!(users.len(), 1, "expected one user turn, got {:?}", users);
        assert!(users[0].text.contains("优化"));
        assert!(!users[0].text.contains("rgb:"));

        let agent: String = turns
            .iter()
            .filter(|t| t.role == "agent")
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(agent.contains("优化空间"), "agent text: {}", agent);
        assert!(!agent.contains("Globbing"), "agent text: {}", agent);
        assert!(!agent.contains("Grepping"), "agent text: {}", agent);
    }
}
