use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::pty::content_filter::{filter_content_lines, is_noise_line};
use crate::pty::terminal_input::{
    extract_raw_submissions, parse_submitted_lines, strip_terminal_escapes,
};
use crate::pty::PtyEvent;
use std::io::Write;

use crate::pty::vt100_recorder::{
    lines_added, stdout_has_ansi, unique_content_lines, ScreenSnapshot, Vt100Recorder,
};
use crate::storage::StreamWriter;

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
    merge_consecutive_turns(turns)
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
        return (snapshots, merge_consecutive_turns(turns));
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

    (snapshots, merge_consecutive_turns(turns))
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

/// 合并相邻同角色 turn（例如人类粘贴多行导致的拆分），避免 chat 显示中一段内容被拆成多个块。
fn merge_consecutive_turns(turns: Vec<ConversationTurn>) -> Vec<ConversationTurn> {
    if turns.len() <= 1 {
        return turns;
    }
    let mut out = Vec::with_capacity(turns.len());
    let mut cur = turns[0].clone();
    for t in turns.into_iter().skip(1) {
        if t.role == cur.role {
            if !cur.text.trim_end().is_empty() && !t.text.trim_start().is_empty() {
                cur.text.push_str("\n\n");
            }
            cur.text.push_str(&t.text);
        } else {
            out.push(cur);
            cur = t;
        }
    }
    out.push(cur);
    out
}

/// 检测单行是否为典型的 Agent 工具/状态活动（用于折叠）。
fn is_tool_activity_line(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 4 {
        return true;
    }
    let lower = t.to_lowercase();
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
        // 额外覆盖真实会话中泄漏的 gh/CI/Waited 片段（即使上游 filter 版本较旧的存档）
        || lower.starts_with("waited for")
        || lower.contains("waited for ")
        || lower.contains("grok bui")
        || lower.contains(" in shell")
        || lower.contains("\"conclusion\"")
        || lower.contains("\"createdat\"")
        || lower.contains("complete|error")
        || lower.contains("uploading|bundling")
        || (lower.contains("globbed") && lower.contains("read"))
        || (lower.contains("grepped") && lower.contains("read"))
        || (t.len() < 60 && lower.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit()) && lower.contains("background"))
        || lower.contains("… trunca")
        || lower.starts_with("your br")
        // 拆分的 gh actions 片段
        || (lower.contains("/actions/r") && t.len() < 150)
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

/// 运行时增量对话提取器。
///
/// 在 PTY 捕获线程中持续喂入 stdout chunk 和 stdin 提交事件，
/// 实时维护 vt100 快照 + 已知 user prompt 边界，
/// 并在每个用户提交时立即将“上一个 agent 响应 + 当前 user prompt”
/// 序列化为 JSONL 追加到 conversation / normalized 流（如果提供了 writer）。
///
/// 目标：把原来 exit 时 O(全量历史) 的重提取工作，分散到会话进行中完成，
/// 使 `il run` 结束时的保存时间接近 O(1) 或 O(最后几轮)。
pub struct LiveConversationTracker {
    recorder: Vt100Recorder,
    prompts: Vec<String>,
    /// 下一个 agent 响应开始的快照索引（随已完成 turn 推进）
    next_agent_start: usize,
    /// 已完成并写出的 turn 数量（用于最终 finalize 时的剩余处理）
    emitted_turn_count: usize,
    /// 是否在 feed 中见过 ANSI（用于决定提取策略）
    has_ansi: bool,
    conv_writer: Option<StreamWriter>,
    norm_writer: Option<StreamWriter>,
    last_emitted_norm_len: usize,
    /// 累积的已清理 stdin（仅用户输入，体积很小），用于实时增量 parse_submitted_lines
    stdin_cleaned: Vec<u8>,
    last_parsed_prompt_count: usize,

    /// 等待“出现在 stdout 快照中”的用户输入候选。
    /// 只有当用户真正输入的内容被 TUI 渲染到屏幕上（screen_contains_prompt 能找到），
    /// 我们才真正把它当作一次对话 user turn 持久化。
    /// 这能有效过滤掉 TUI 内部状态、CI 日志片段、时间戳标签等噪音。
    pending_user_candidates: Vec<(String, std::time::Instant)>,
}

impl LiveConversationTracker {
    pub fn new(
        rows: u16,
        cols: u16,
        conv: Option<StreamWriter>,
        norm: Option<StreamWriter>,
    ) -> Self {
        Self {
            recorder: Vt100Recorder::new(rows, cols),
            prompts: Vec::new(),
            next_agent_start: 0,
            emitted_turn_count: 0,
            has_ansi: false,
            conv_writer: conv,
            norm_writer: norm,
            last_emitted_norm_len: 0,
            stdin_cleaned: Vec::new(),
            last_parsed_prompt_count: 0,
            pending_user_candidates: Vec::new(),
        }
    }

    /// 持续喂入 stdout 原始字节，内部驱动 vt100 parser + 按变化产生快照。
    /// 同时增量把新产生的 normalized 快照写出（如果 writer 存在）。
    pub fn feed_stdout(&mut self, data: &[u8]) {
        if !self.has_ansi && data.contains(&0x1b) {
            self.has_ansi = true;
        }
        let before = self.recorder.snapshots().len();
        self.recorder.feed(data);
        let after = self.recorder.snapshots().len();

        if let Some(w) = &mut self.norm_writer {
            for snap in &self.recorder.snapshots()[before..after] {
                if let Ok(line) = serde_json::to_string(snap) {
                    let mut s = line;
                    s.push('\n');
                    let _ = w.write_all(s.as_bytes());
                }
            }
            self.last_emitted_norm_len = after;
        }

        // 新快照到达后，尝试把 pending 的用户输入提升（很多 TUI 是在输出阶段才把用户消息渲染上去的）
        if after > before {
            self.promote_pending_candidates();
        }
    }

    /// 接收原始 stdin 字节（来自 PTY stdin 捕获线程）。
    /// 仅收集为“pending candidate”，不在此时立即写出。
    /// 真正的 user turn 只有在 feed_stdout 后续快照中看到该文本被渲染时才会真正 emit。
    pub fn feed_stdin_raw(&mut self, data: &[u8]) {
        let cleaned = strip_terminal_escapes(data);
        self.stdin_cleaned.extend_from_slice(&cleaned);

        let all = parse_submitted_lines(&self.stdin_cleaned);
        if all.len() > self.last_parsed_prompt_count {
            let now = std::time::Instant::now();
            for p in &all[self.last_parsed_prompt_count..] {
                if !p.trim().is_empty() {
                    self.pending_user_candidates.push((p.clone(), now));
                }
            }
            self.last_parsed_prompt_count = all.len();
        }

        // 尝试把已经出现在最新屏幕上的候选提升为真实 user turn
        self.promote_pending_candidates();
    }

    /// 检查 pending 候选是否已出现在最近的 stdout 快照中。
    /// 出现则立即提升为正式 user turn（写出 + 闭合上一段 agent 响应）。
    fn promote_pending_candidates(&mut self) {
        if self.pending_user_candidates.is_empty() {
            return;
        }

        let snaps = self.recorder.snapshots();
        if snaps.is_empty() {
            return;
        }

        // 只看最近的若干快照，避免全量扫描（性能）
        let recent_start = snaps.len().saturating_sub(40);
        let recent = &snaps[recent_start..];

        let now = std::time::Instant::now();
        let mut still_pending = Vec::new();
        let mut to_promote = Vec::new();

        for (prompt, first_seen) in self.pending_user_candidates.drain(..) {
            // 太久没出现（>10s），认为是内部噪音或非对话输入，直接丢弃
            if now.duration_since(first_seen).as_secs() > 10 {
                continue;
            }

            let visible = recent
                .iter()
                .any(|s| screen_contains_prompt(&s.contents, &prompt));

            if visible {
                to_promote.push(prompt);
            } else {
                still_pending.push((prompt, first_seen));
            }
        }

        self.pending_user_candidates = still_pending;

        for p in to_promote {
            self.accept_user_prompt(p);
        }
    }

    /// 内部：真正接受一个用户 prompt 并写出 user turn + 可能的上一段 agent 响应。
    /// （原 on_user_submit 的核心逻辑，现由 pending 机制调用）
    fn accept_user_prompt(&mut self, prompt: String) {
        if prompt.trim().is_empty() {
            return;
        }
        let now = Utc::now().to_rfc3339();

        let is_first = self.prompts.is_empty();
        self.prompts.push(prompt.clone());

        // 写出本轮 user turn
        let user_turn = ConversationTurn {
            role: "user".to_string(),
            text: prompt.clone(),
            ts: now.clone(),
        };
        self.append_turn(&user_turn);

        if !is_first {
            // 闭合上一个 prompt 对应的 agent 响应
            let prev_idx = self.prompts.len() - 2;
            let prev_prompt = &self.prompts[prev_idx];
            let next_prompt = &prompt;

            let snaps = self.recorder.snapshots();
            let mut j = self.next_agent_start;
            while j < snaps.len() && !screen_contains_prompt(&snaps[j].contents, next_prompt) {
                j += 1;
            }
            let agent_end = j.saturating_sub(1).max(self.next_agent_start);

            let agent_text =
                self.extract_agent_text(self.next_agent_start, agent_end, &[prev_prompt.as_str()]);
            if !agent_text.is_empty() {
                let agent_turn = ConversationTurn {
                    role: "agent".to_string(),
                    text: agent_text,
                    ts: now,
                };
                self.append_turn(&agent_turn);
            }

            self.next_agent_start = agent_end.saturating_add(1);
        }

        self.emitted_turn_count = self.prompts.len();
    }

    /// 会话结束时调用：把最后一个（可能仍在进行中的）agent 响应写出。
    /// 如果没有任何 user prompt，则把整个输出作为单个 agent turn 写出。
    pub fn finalize_last_turn(&mut self) {
        self.recorder.force_snapshot();
        // 最后再尝试提升剩余 pending（有些 prompt 可能在最后几帧才被渲染）
        self.promote_pending_candidates();

        let snaps = self.recorder.snapshots();
        if snaps.is_empty() {
            return;
        }

        let now = Utc::now().to_rfc3339();

        if self.prompts.is_empty() {
            // 无任何输入，整段 stdout 都是 agent 的一次性输出
            let text = self.extract_agent_text(0, snaps.len().saturating_sub(1), &[]);
            if !text.is_empty() {
                let turn = ConversationTurn {
                    role: "agent".to_string(),
                    text,
                    ts: now,
                };
                self.append_turn(&turn);
            }
            return;
        }

        // 有 prompt，最后一个 agent 响应从 next_agent_start 到结尾
        if self.next_agent_start < snaps.len() {
            let last_prompt = self.prompts.last().map(|s| s.as_str()).unwrap_or("");
            let text = self.extract_agent_text(
                self.next_agent_start,
                snaps.len().saturating_sub(1),
                &[last_prompt],
            );
            if !text.is_empty() {
                let turn = ConversationTurn {
                    role: "agent".to_string(),
                    text,
                    ts: now,
                };
                self.append_turn(&turn);
            }
        }
    }

    /// 返回当前已积累的所有快照（供测试或最终批处理回退）。
    pub fn snapshots(&self) -> &[ScreenSnapshot] {
        self.recorder.snapshots()
    }

    /// 返回已收集到的 user prompts（调试/测试用）。
    pub fn prompts(&self) -> &[String] {
        &self.prompts
    }

    fn append_turn(&mut self, turn: &ConversationTurn) {
        if let Some(w) = &mut self.conv_writer {
            if let Ok(line) = serde_json::to_string(turn) {
                let mut s = line;
                s.push('\n');
                let _ = w.write_all(s.as_bytes());
            }
        }
    }

    fn extract_agent_text(&self, start: usize, end: usize, user_prompts: &[&str]) -> String {
        let snaps = self.recorder.snapshots();
        if snaps.is_empty() {
            return String::new();
        }
        let mut lines = lines_between_snapshots(snaps, start, end);
        lines.retain(|line| {
            !user_prompts
                .iter()
                .any(|p| line.trim() == p.trim() || line.contains(p.trim()))
        });
        filter_content_lines(lines).join("\n")
    }
}

/// Format conversation turns into a human-friendly chat view (il dump chat 推荐输出)。
///
/// 设计目标：仅提炼最核心的用户-代理交互。
/// - Human 输入完整保留
/// - Agent 最终回答完整保留
/// - Agent 内部工具调用、思考轨迹、状态日志自动折叠为摘要
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
            let role_label = if is_user { "Human" } else { "Agent" };
            // Bold + color (cyan for user, green for agent). Works in most modern terminals.
            let color = if is_user { "\x1b[1;36m" } else { "\x1b[1;32m" };
            let reset = "\x1b[0m";

            // 时间戳当前为提取时刻的统一值，对话内各 turn 实际发生时间无法精确还原；为避免视觉重复与误导，默认不在 chat 视图展示。
            out.push_str(&format!("{}{}{}\n", color, role_label, reset));

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

/// 从原始 stdin 流重建“用户实际提交”的完整输入时间线。
///
/// 这是目前最可靠的 Human 输入还原手段（独立于 vt100 屏幕匹配和 conversation 启发式）。
/// 支持 bracketed paste 多行输入、基本退格编辑。
///
/// 推荐用法：`il dump timeline`（或内部调用此函数 + 后续丰富时间戳）。
pub fn format_raw_input_timeline(stdin: &[u8]) -> String {
    let subs = extract_raw_submissions(stdin);
    if subs.is_empty() {
        return "（未从 stdin 中提取到可识别的用户提交）".to_string();
    }

    let mut out = String::new();
    for (idx, sub) in subs.iter().enumerate() {
        let marker = if sub.from_bracketed_paste {
            " [bracketed-paste]"
        } else {
            ""
        };
        let size = sub.raw_end.saturating_sub(sub.raw_start);
        out.push_str(&format!(
            "=== Human Input #{} ({} bytes{}) ===\n",
            idx + 1,
            size,
            marker
        ));
        out.push_str(&sub.text);
        out.push_str("\n\n");
    }
    out.trim_end().to_string()
}

/// 带时间戳 + stdout 上下文的增强 timeline（推荐用于 `il dump timeline`）。
///
/// 利用 PtyInput / PtyOutput 事件中新增的 cumulative `offset` 字段，
/// 将 RawSubmission 的原始字节区间精确映射回真实墙钟时间。
/// 同时报告每个提交时刻 stdout 已经累积的字节数，作为“输出上下文”锚点。
///
/// 旧会话（事件无 offset）会优雅退化到无时间戳版本。
pub fn format_timeline_annotated(stdin: &[u8], events_jsonl: &[u8]) -> String {
    let subs = extract_raw_submissions(stdin);
    if subs.is_empty() {
        return "（未从 stdin 中提取到可识别的用户提交）".to_string();
    }

    // 解析所有事件，只保留带 offset 的 PtyInput / PtyOutput
    let mut input_events: Vec<(u64, String)> = Vec::new(); // (offset, ts)
    let mut output_offsets: Vec<u64> = Vec::new(); // 仅用于“附近 stdout 字节数”

    let text = String::from_utf8_lossy(events_jsonl);
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<PtyEvent>(line) {
            if let Some(off) = ev.offset {
                match ev.kind.as_str() {
                    "PtyInput" => input_events.push((off, ev.ts)),
                    "PtyOutput" => output_offsets.push(off),
                    _ => {}
                }
            }
        }
    }

    // 按 offset 排序（同一流内 offset 单调递增）
    input_events.sort_by_key(|&(o, _)| o);
    output_offsets.sort_unstable();

    let mut out = String::new();
    for (idx, sub) in subs.iter().enumerate() {
        let marker = if sub.from_bracketed_paste {
            " [bracketed-paste]"
        } else {
            ""
        };
        let size = sub.raw_end.saturating_sub(sub.raw_start);

        // 找最接近这次提交的 PtyInput 事件（第一个 offset >= sub 起始）
        let ts = input_events
            .iter()
            .find(|&&(off, _)| off >= sub.raw_start as u64)
            .map(|(_, t)| t.as_str())
            .unwrap_or("");

        let ts_prefix = if ts.is_empty() {
            String::new()
        } else {
            format!(" @ {}", ts)
        };

        // 找该时刻最近的 stdout 累积字节（作为轻量“输出上下文”）
        let stdout_ctx = if let Some(&last_out) = output_offsets
            .iter()
            .rev()
            .find(|&&o| o <= sub.raw_start as u64 + size as u64)
        {
            format!(" (stdout~{}B)", last_out)
        } else {
            String::new()
        };

        out.push_str(&format!(
            "=== Human Input #{} ({} bytes{}{}){} ===\n",
            idx + 1,
            size,
            marker,
            stdout_ctx,
            ts_prefix
        ));
        out.push_str(&sub.text);
        out.push_str("\n\n");
    }

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

        assert!(pretty.contains("Human"));
        assert!(pretty.contains("Agent"));
        assert!(pretty.contains("帮我优化这个函数"));
        assert!(pretty.contains("hot path"));
        // Should not contain raw JSON structure
        assert!(!pretty.contains("\"role\""));
        assert!(!pretty.contains("\"text\""));
        // 时间戳不再默认显示（提取时统一，无法区分各 turn 实际时刻）
    }

    #[test]
    fn fold_internal_thoughts_collapses_real_agent_noise() {
        // 模拟从 conversation turn.text 反序列化后得到的带真实换行的 agent 文本（包含 Grok/Cursor 真实痕迹 + gh/CI 监控片段）
        let noisy = "Cursor Agent\nv2026.05.28\nGrok Build 0.1 1M\nGlobbing, grepping 2 globs, 1 grep\nGlobbed, grepped 2 globs, 1 grep\nTo-do Working on 4 to-dos\n☐ 监控\nWaiting 2m for shell\nMonitored background task\nRan ls\nWebFetch https://...\n{\"status\":\"in_progress\"}\nWaited for \"Success|Published|https://intentloop.dev|De\n2 background\ncomplete|Error|failed|✨|Uploading|Bundling|Finished\n[{\"conclusion\":\"\",\"createdAt\":\"2026-05-31T05:54\"}\n我已经完成了所有修改，核心是更激进过滤+折叠。\n额外的一点说明。";

        let folded = fold_internal_thoughts(noisy);

        assert!(folded.contains("我已经完成了所有修改"));
        assert!(folded.contains("核心是更激进过滤+折叠"));
        assert!(folded.contains("额外的一点说明"));

        // 噪音全部消失（包括新观察到的 gh/CI/Waited 片段）
        assert!(!folded.contains("Globbing, grepping"));
        assert!(!folded.contains("Grok Build"));
        assert!(!folded.contains("To-do Working"));
        assert!(!folded.contains("☐"));
        assert!(!folded.contains("Waiting 2m"));
        assert!(!folded.contains("Monitored background"));
        assert!(!folded.contains("WebFetch"));
        assert!(!folded.contains("in_progress"));
        assert!(!folded.contains("Waited for"));
        assert!(!folded.contains("complete|Error"));
        assert!(!folded.contains("conclusion"));
        assert!(!folded.contains("2 background"));

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

        assert!(pretty.contains("Human"));
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

    #[test]
    fn extract_raw_submissions_handles_bracketed_paste_multi_line() {
        // 模拟真实多行 paste + 提交：bracketed paste 包裹多行文本，最后 \r 提交
        let raw_stdin: Vec<u8> = [
            b"\x1b[200~".as_slice(),
            "针对 IntentLoop 多行输入问题\n我们需要综合 stdin stdout events\n来完整还原用户真正提交的内容".as_bytes(),
            b"\x1b[201~\r".as_slice(),
        ]
        .concat();

        let subs = extract_raw_submissions(&raw_stdin);

        assert_eq!(subs.len(), 1);
        let s = &subs[0];
        assert!(s.from_bracketed_paste);
        assert!(s.text.contains("针对 IntentLoop 多行输入问题"));
        assert!(s.text.contains("完整还原用户真正提交的内容"));
        assert!(s.raw_start < s.raw_end);
    }

    #[test]
    fn format_timeline_annotated_attaches_ts_from_ptyinput_events() {
        // 构造一个带 bracketed paste 的 stdin + 对应的 PtyInput 事件 JSONL（带 offset）
        let raw_stdin: Vec<u8> = [
            b"\x1b[200~".as_slice(),
            "多行问题\n用事件 offset 精确关联时间\n".as_bytes(),
            b"\x1b[201~\r".as_slice(),
        ]
        .concat();

        // 模拟事件流：一个 PtyInput 在 offset 30 处（paste 内容大致范围）
        let events_jsonl = r#"{"type":"PtyInput","ts":"2026-05-31T18:05:00.123Z","bytes":45,"offset":45}
{"type":"PtyOutput","ts":"2026-05-31T18:05:00.200Z","bytes":120,"offset":120}
"#;

        let annotated = format_timeline_annotated(&raw_stdin, events_jsonl.as_bytes());

        assert!(annotated.contains("Human Input #1"));
        assert!(annotated.contains("2026-05-31T18:05:00.123Z")); // 拿到了事件时间
        assert!(annotated.contains("bracketed-paste"));
        // stdout 上下文是尽力而为（取决于 offset 是否落入当前 submission 范围），核心价值已由时间戳证明
        assert!(annotated.contains("多行问题"));
        assert!(annotated.contains("精确关联时间"));
    }
}
