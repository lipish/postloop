use chrono::Utc;
use serde::Serialize;

use crate::pty::content_filter::filter_content_lines;
use crate::pty::terminal_input::{parse_submitted_lines, strip_terminal_escapes};
use crate::pty::vt100_recorder::{
    lines_added, stdout_has_ansi, unique_content_lines, ScreenSnapshot, Vt100Recorder,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversationTurn {
    pub role: String,
    pub text: String,
    pub ts: String,
}

/// 从 PTY 原始 stdout/stdin 提取对话：stdout 走 vt100 屏幕快照，stdin 走行编辑回放。
pub fn extract_conversation(
    stdout: &[u8],
    stdin: &[u8],
    rows: u16,
    cols: u16,
) -> Vec<ConversationTurn> {
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
        let text = agent_text_from_snapshots(
            stdout,
            &snapshots,
            0,
            snapshots.len().saturating_sub(1),
            &[],
        );
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
        let text =
            agent_text_from_snapshots(stdout, &snapshots, agent_start, agent_end, &[prompt_str]);
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
    lines.retain(|line| {
        !user_prompts
            .iter()
            .any(|p| line.trim() == p.trim() || line.contains(p.trim()))
    });
    filter_content_lines(lines).join("\n")
}

fn lines_between_snapshots(
    snapshots: &[ScreenSnapshot],
    start_idx: usize,
    end_idx: usize,
) -> Vec<String> {
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
        .filter_map(|t| serde_json::to_string(t).ok())
        .collect()
}

pub fn snapshots_to_jsonl(snapshots: &[ScreenSnapshot]) -> Vec<String> {
    snapshots
        .iter()
        .filter_map(|s| serde_json::to_string(s).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt100_extracts_mock_sqlite_answer() {
        // Mock stdin: user inputs "What is SQLite?\n"
        let stdin = b"What is SQLite?\n";

        // Mock stdout: ANSI sequences, user prompt echoed, then agent response with "SQLite" and "嵌入式"
        let stdout = b"\x1b[2J\x1b[HWhat is SQLite?\r\nSQLite is a lightweight \xe5\xb5\x8c\xe5\x85\xa5\xe5\xbc\x8f database engine.\r\n";

        let turns = extract_conversation(stdout, stdin, 24, 80);

        let user: Vec<_> = turns.iter().filter(|t| t.role == "user").collect();
        assert_eq!(user.len(), 1);
        assert_eq!(user[0].text, "What is SQLite?");

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
    fn filters_agent_tui_noise_from_mock_session() {
        // Mock stdin: user inputs "优化代码\n"
        let stdin = b"\x1b]11;rgb:ffff/fcfc/f0f0\x07\x1b[Is\x7f\xe4\xbc\x98\xe5\x8c\x96\xe4\xbb\xa3\xe7\xa0\x81\n";

        // Mock stdout contains TUI noise like Globbing/Grepping, and then "优化空间"
        let stdout = b"Globbing \"**/*\" in .\r\nGrepping for keywords...\r\n\xe4\xbc\x98\xe5\x8c\x96\xe7\xa0\x81\r\n\xe4\xbc\x98\xe5\x8c\x96\xe7\xa9\xba\xe9\x97\xb4 is huge.\r\n";

        let turns = extract_conversation(stdout, stdin, 24, 80);

        let user: Vec<_> = turns.iter().filter(|t| t.role == "user").collect();
        assert_eq!(user.len(), 1);
        assert!(user[0].text.contains("优化"));
        assert!(!user[0].text.contains("rgb:"));

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
