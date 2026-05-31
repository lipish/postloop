use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::pty::content_filter::{filter_content_lines, is_noise_line};
use crate::pty::terminal_input::{parse_submitted_lines, strip_terminal_escapes};
use crate::pty::vt100_recorder::{
    lines_added, stdout_has_ansi, unique_content_lines, ScreenSnapshot, Vt100Recorder,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
                ts: Utc::now().to_rfc3339(),
            });
        }
        return (snapshots, turns);
    }

    let mut agent_start = 0usize;
    for (pi, prompt) in user_prompts.iter().enumerate() {
        turns.push(ConversationTurn {
            role: "user".to_string(),
            text: prompt.clone(),
            ts: Utc::now().to_rfc3339(),
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
                ts: Utc::now().to_rfc3339(),
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

/// 检测单行是否为典型的 Agent 工具/状态活动（用于折叠）。
fn is_tool_activity_line(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 4 {
        return true;
    }
    is_noise_line(t)
        || t.starts_with("Globbing")
        || t.starts_with("Grepping")
        || t.starts_with("Reading")
        || t.starts_with("Writing")
        || t.starts_with("Executing")
        || t.contains(" in .") && t.len() < 90
        || t.starts_with("Ran ")
        || t.starts_with("WebFetch")
        || t.starts_with("cat ")
}

/// 将 Agent 文本中的连续工具/思考痕迹折叠为单行摘要。
/// 提炼核心交互：保留用户输入 + Agent 最终回答，工具调用以摘要形式记录。
fn fold_internal_thoughts(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let mut tool_run = 0usize;
    let mut verbs_seen: std::collections::HashSet<&str> = std::collections::HashSet::new();

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if is_tool_activity_line(line) || (trimmed.len() < 6 && !trimmed.is_empty()) {
            tool_run += 1;
            // 粗略统计动作类型
            let l = trimmed.to_lowercase();
            for v in [
                "glob", "grep", "read", "write", "exec", "fetch", "plan", "build", "search",
            ] {
                if l.contains(v) {
                    verbs_seen.insert(v);
                }
            }
            i += 1;
            continue;
        }

        if tool_run >= 2 {
            let summary = if verbs_seen.is_empty() {
                format!("[Agent 内部操作已折叠（{} 行工具/状态日志）]", tool_run)
            } else {
                let vs: Vec<_> = verbs_seen.iter().copied().collect();
                format!(
                    "[Agent 内部操作已折叠（{} 行，涉及 {} 等工具调用）]",
                    tool_run,
                    vs.join("/")
                )
            };
            out.push(summary);
            verbs_seen.clear();
            tool_run = 0;
        } else if tool_run > 0 {
            // 短的连续噪声，丢弃（不值得单独显示）
            tool_run = 0;
            verbs_seen.clear();
        }

        if !trimmed.is_empty() {
            out.push(trimmed.to_string());
        }
        i += 1;
    }

    // 尾部工具块
    if tool_run >= 2 {
        let summary = if verbs_seen.is_empty() {
            format!("[Agent 内部操作已折叠（{} 行工具/状态日志）]", tool_run)
        } else {
            let vs: Vec<_> = verbs_seen.iter().copied().collect();
            format!(
                "[Agent 内部操作已折叠（{} 行，涉及 {} 等工具调用）]",
                tool_run,
                vs.join("/")
            )
        };
        out.push(summary);
    }

    out.join("\n")
}

/// Format conversation turns into a human-friendly chat view (il dump chat 推荐输出)。
///
/// 设计目标：仅提炼最核心的用户-代理交互。
/// - User 输入完整保留
/// - Agent 最终回答完整保留
/// - Agent 内部工具调用、思考轨迹、状态日志（Globbing/Grepping/To-do/Grok Build 等）自动折叠为摘要
/// - 工具使用以计数形式记录在折叠说明中（详细原始轨迹请用 `il dump thoughts` 或 `il dump stdout`）
///
/// 输出特点：
/// - 彩色角色标签 + 短时间戳
/// - 无 JSON、无噪音、无重复状态行
pub fn format_conversation_chat(jsonl: &[u8]) -> String {
    let mut out = String::new();
    let text = String::from_utf8_lossy(jsonl);

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(turn) = serde_json::from_str::<ConversationTurn>(line) {
            let is_user = turn.role == "user";
            let role_label = if is_user { "User" } else { "Agent" };
            // Bold + color (cyan for user, green for agent). Works in most modern terminals.
            let color = if is_user { "\x1b[1;36m" } else { "\x1b[1;32m" };
            let reset = "\x1b[0m";

            // Extract HH:MM from RFC3339 timestamp when possible
            let short_ts = turn
                .ts
                .get(11..16)
                .map(|s| format!(" ({})", s))
                .unwrap_or_default();

            out.push_str(&format!("{}{}{}{}\n", color, role_label, short_ts, reset));

            let raw_body = turn.text.trim();
            let body = if is_user {
                raw_body.to_string()
            } else {
                fold_internal_thoughts(raw_body)
            };

            if !body.is_empty() {
                out.push_str(&body);
                out.push_str("\n\n");
            }
        }
    }

    // Trim trailing blank lines
    out.trim_end().to_string()
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

    #[test]
    fn format_conversation_chat_produces_readable_output() {
        // Simulate JSONL that would come from a stored conversation stream
        let jsonl = r#"{"role":"user","text":"帮我优化这个函数","ts":"2026-05-31T10:22:00Z"}
{"role":"agent","text":"好的，我看了代码。主要问题在 hot path 里。","ts":"2026-05-31T10:22:05Z"}
"#
        .as_bytes();

        let pretty = format_conversation_chat(jsonl);

        assert!(pretty.contains("User"));
        assert!(pretty.contains("Agent"));
        assert!(pretty.contains("帮我优化这个函数"));
        assert!(pretty.contains("hot path"));
        // Should not contain raw JSON structure
        assert!(!pretty.contains("\"role\""));
        assert!(!pretty.contains("\"text\""));
        // Should contain short time
        assert!(pretty.contains("(10:22)"));
    }

    #[test]
    fn fold_internal_thoughts_collapses_real_agent_noise() {
        // 模拟从 conversation turn.text 反序列化后得到的带真实换行的 agent 文本（包含 Grok/Cursor 真实痕迹）
        let noisy = "Cursor Agent\nv2026.05.28\nGrok Build 0.1 1M\nGlobbing, grepping 2 globs, 1 grep\nGlobbed, grepped 2 globs, 1 grep\nTo-do Working on 4 to-dos\n☐ 监控\nWaiting 2m for shell\nMonitored background task\nRan ls\nWebFetch https://...\n{\"status\":\"in_progress\"}\n我已经完成了所有修改，核心是更激进过滤+折叠。\n额外的一点说明。";

        let folded = fold_internal_thoughts(noisy);

        assert!(folded.contains("我已经完成了所有修改"));
        assert!(folded.contains("核心是更激进过滤+折叠"));
        assert!(folded.contains("额外的一点说明"));

        // 噪音全部消失
        assert!(!folded.contains("Globbing, grepping"));
        assert!(!folded.contains("Grok Build"));
        assert!(!folded.contains("To-do Working"));
        assert!(!folded.contains("☐"));
        assert!(!folded.contains("Waiting 2m"));
        assert!(!folded.contains("Monitored background"));
        assert!(!folded.contains("WebFetch"));
        assert!(!folded.contains("in_progress"));

        // 出现折叠摘要
        assert!(folded.contains("[Agent 内部操作已折叠"));
        assert!(folded.contains("glob") || folded.contains("工具"));
    }

    #[test]
    fn format_conversation_chat_folds_grok_cursor_tool_noise() {
        // 使用 serde 正确构造带转义 \n 的 JSONL，模拟真实存储的 conversation 流
        let turns = vec![
            ConversationTurn {
                role: "user".into(),
                text: "为什么还是很乱？".into(),
                ts: "2026-05-31T14:50:00Z".into(),
            },
            ConversationTurn {
                role: "agent".into(),
                text: "Cursor Agent\nv2026\nGrok Build 0.1\nGlobbing, grepping\nTo-do Working on 3\n☐ foo\nWaiting for shell\n我已用更激进的过滤完成优化。".into(),
                ts: "2026-05-31T14:50:10Z".into(),
            },
        ];
        let jsonl_bytes = turns_to_jsonl(&turns).join("\n").into_bytes();

        let pretty = format_conversation_chat(&jsonl_bytes);

        assert!(pretty.contains("User"));
        assert!(pretty.contains("Agent"));
        assert!(pretty.contains("为什么还是很乱"));
        assert!(pretty.contains("我已用更激进的过滤完成优化"));

        // 无任何噪音
        assert!(!pretty.contains("Globbing"));
        assert!(!pretty.contains("Grok Build"));
        assert!(!pretty.contains("To-do"));
        assert!(!pretty.contains("☐"));
        assert!(!pretty.contains("Waiting for shell"));

        // 有折叠提示（记录了工具调用）
        assert!(pretty.contains("[Agent 内部操作已折叠"));
    }
}
