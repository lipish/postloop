use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::conversation;
use crate::pty::{CaptureWriter, CompatPtySession, PtyEvent};
use crate::registry::Registry;

use anyhow::anyhow;

pub struct ExecutionOutput {
    pub stdout: String,
    pub stdout_bytes: Vec<u8>,
    pub stderr: String,
    pub stdin_log: String,
    pub stdin_bytes: Vec<u8>,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub structured_events: Vec<String>,
    pub conversation: Vec<String>,
    pub normalized: Vec<String>,
    pub ring_buffer: Vec<u8>,
}

pub fn run_session(
    repo_root: PathBuf,
    command: Vec<String>,
    interactive: bool,
    extra_env: &HashMap<String, String>,
) -> Result<(), anyhow::Error> {
    let registry = Registry::init(&repo_root)?;

    let session_id = Uuid::now_v7().to_string();
    let log_ref = format!("memmap_fs:sessions/{}/stdout", session_id);

    let agent_cmd = command.join(" ");
    registry.create_session(&session_id, &agent_cmd, &repo_root, &log_ref)?;

    println!("▶ Running session: {}", session_id);
    println!("Command: {}", agent_cmd);
    if interactive {
        println!("Mode: PTY interactive");
    }

    let execution = if interactive {
        execute_with_pty(&registry, &session_id, &repo_root, &command, extra_env)?
    } else {
        let execution = execute_non_interactive(&repo_root, &command, extra_env)?;
        registry.append_stream(&session_id, "stdout", &execution.stdout_bytes)?;
        registry.append_stream(&session_id, "stderr", execution.stderr.as_bytes())?;
        execution
    };

    if !execution.structured_events.is_empty() {
        append_jsonl_stream(
            &registry,
            &session_id,
            "events",
            &execution.structured_events,
        )?;
        println!(
            "Structured events: memmap_fs:sessions/{}/events",
            session_id
        );
    }

    // 旧的 append 路径（仅 batch 提取时有内容）。live tracker 已在用户输入期间实时写入对应 stream。
    if !execution.normalized.is_empty() {
        append_jsonl_stream(&registry, &session_id, "normalized", &execution.normalized)?;
    }
    if !execution.conversation.is_empty() {
        append_jsonl_stream(
            &registry,
            &session_id,
            "conversation",
            &execution.conversation,
        )?;
    }

    // 无论 batch 还是 live，运行结束后若对应流非空就提示（live 模式下内容已在过程中持久化）
    if !registry
        .read_stream_to_bytes(&session_id, "normalized")
        .unwrap_or_default()
        .is_empty()
    {
        println!(
            "VT100 normalized: memmap_fs:sessions/{}/normalized",
            session_id
        );
    }
    if !registry
        .read_stream_to_bytes(&session_id, "conversation")
        .unwrap_or_default()
        .is_empty()
    {
        println!(
            "Conversation log: memmap_fs:sessions/{}/conversation",
            session_id
        );
    }

    if !execution.ring_buffer.is_empty() {
        registry.append_stream(&session_id, "ring", &execution.ring_buffer)?;
        println!("Ring buffer: memmap_fs:sessions/{}/ring", session_id);
    }

    let stdout_lines: Vec<String> = execution.stdout.lines().map(str::to_string).collect();
    let stderr_lines: Vec<String> = execution.stderr.lines().map(str::to_string).collect();
    let stdin_lines: Vec<String> = execution.stdin_log.lines().map(str::to_string).collect();

    let mut seq = 1i64;
    let mut thought_count = 0i64;
    if !stdin_lines.is_empty() {
        let (next_seq, added) =
            registry.add_thought_events(&session_id, "stdin", &stdin_lines, seq)?;
        seq = next_seq;
        thought_count += added;
    }
    let (next_seq, added) =
        registry.add_thought_events(&session_id, "stdout", &stdout_lines, seq)?;
    seq = next_seq;
    thought_count += added;
    let (_, added) = registry.add_thought_events(&session_id, "stderr", &stderr_lines, seq)?;
    thought_count += added;
    registry.set_thought_count(&session_id, thought_count)?;

    let status = if execution.success {
        "succeeded"
    } else {
        "failed"
    };
    registry.complete_session(&session_id, status, execution.exit_code)?;

    // Index conversation for full-text search
    if !execution.conversation.is_empty() {
        if let Err(e) = registry.index_conversation(&session_id, &execution.conversation) {
            eprintln!("Warning: Failed to index conversation: {}", e);
        }
    }

    let report = generate_min_report(&registry, &session_id)?;
    if !report.is_empty() {
        registry.append_stream(&session_id, "report", report.as_bytes())?;
        println!("Report: memmap_fs:sessions/{}/report", session_id);
    }

    println!("✓ Session saved: {}", session_id);
    println!();
    println!("Quick commands:");
    println!("  il last   # 查看最近会话（推荐）");
    println!("  il show   # 同上，默认隐式使用最新会话");
    println!("  il list   # 列出历史会话");

    if !execution.success {
        return Err(anyhow!(
            "Agent command exited with status {}",
            execution
                .exit_code
                .map(|v| v.to_string())
                .unwrap_or_else(|| "terminated by signal".to_string())
        ));
    }

    Ok(())
}

fn append_jsonl_stream(
    registry: &Registry,
    session_id: &str,
    stream: &str,
    lines: &[String],
) -> Result<(), anyhow::Error> {
    if lines.is_empty() {
        return Ok(());
    }

    let mut jsonl = lines.join("\n");
    jsonl.push('\n');
    registry.append_stream(session_id, stream, jsonl.as_bytes())
}

/// 解析会话选择器：支持显式 ID、--last、或隐式默认最新会话。
/// 规则：
/// - 若 use_last 且 session_id 同时提供 → 报错（互斥）
/// - 若 use_last 或 session_id 为 None → 返回最近一次会话 ID
/// - 否则返回给定的 session_id
fn resolve_session_id(
    registry: &Registry,
    session_id: Option<&str>,
    use_last: bool,
) -> Result<String, anyhow::Error> {
    if use_last && session_id.is_some() {
        return Err(anyhow!("--last 与会话 ID 互斥，请只指定其中之一"));
    }

    if use_last || session_id.is_none() {
        return registry
            .get_latest_session()?
            .map(|s| s.id)
            .ok_or_else(|| anyhow!("未找到任何会话。请先运行一次会话。"));
    }

    Ok(session_id.unwrap().to_string())
}

pub fn cmd_run(agent: String, extra_args: Vec<String>) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;

    // 极简策略：永远直接执行用户在当前 shell 环境中已经可以运行的命令。
    // 所有 shell 激活（conda / venv / direnv / asdf / nvm 等）、环境变量、登录态，
    // 均由用户自己的 shell 配置负责。IntentLoop 只负责记录 I/O。
    let mut final_cmd = vec![agent.clone()];
    final_cmd.extend(extra_args);

    println!(
        "▶ Running direct command in {} (full env inherited from your shell)",
        repo_root.display()
    );
    println!("   Command: {}", final_cmd.join(" "));

    // 空 extra_env：子进程完整继承当前父进程环境（即用户终端的真实环境）
    run_session(repo_root, final_cmd, true, &HashMap::new())
}

pub fn cmd_show(session_id: Option<&str>, use_last: bool) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;
    let registry = Registry::init(&repo_root)?;
    let resolved_id = resolve_session_id(&registry, session_id, use_last)?;

    let Some(session) = registry.get_session(&resolved_id)? else {
        return Err(anyhow!("Session not found: {}", resolved_id));
    };

    println!("Session: {}", session.id);
    println!("Status: {}", session.status);
    println!("Started: {}", session.start_at);
    println!(
        "Ended: {}",
        session.end_at.unwrap_or_else(|| "(running)".to_string())
    );
    println!(
        "Exit code: {}",
        session
            .exit_code
            .map(|v| v.to_string())
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!("Command: {}", session.agent_cmd);
    println!("Thought events: {}", session.thought_count);
    println!(
        "Raw stdout stream: memmap_fs:sessions/{}/stdout",
        session.id
    );
    println!("Raw stdin stream: memmap_fs:sessions/{}/stdin", session.id);

    print_stream_ref_if_present(&registry, &resolved_id, "conversation", "Conversation")?;
    print_stream_ref_if_present(&registry, &resolved_id, "events", "Structured events")?;
    print_stream_ref_if_present(&registry, &resolved_id, "normalized", "VT100 normalized")?;
    print_stream_ref_if_present(&registry, &resolved_id, "thoughts", "Thought events")?;
    print_stream_ref_if_present(&registry, &resolved_id, "report", "Report")?;

    if let Ok(ring) = registry.read_stream_to_bytes(&resolved_id, "ring") {
        if !ring.is_empty() {
            println!(
                "Ring buffer: memmap_fs:sessions/{}/ring ({} bytes)",
                resolved_id,
                ring.len()
            );
        }
    }

    Ok(())
}

fn print_stream_ref_if_present(
    registry: &Registry,
    session_id: &str,
    stream: &str,
    label: &str,
) -> Result<(), anyhow::Error> {
    if let Ok(bytes) = registry.read_stream_to_bytes(session_id, stream) {
        if !bytes.is_empty() {
            println!(
                "{}: memmap_fs:sessions/{}/{} ({} bytes)",
                label,
                session_id,
                stream,
                bytes.len()
            );
        }
    }
    Ok(())
}

pub fn cmd_list(limit: usize) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;
    let registry = Registry::init(&repo_root)?;
    let mut sessions = registry.list_sessions()?;

    if sessions.is_empty() {
        println!("No sessions found. Run `il run <your-agent>` to start recording.");
        return Ok(());
    }

    // Newest first
    sessions.sort_by(|a, b| b.start_at.cmp(&a.start_at));

    let shown = sessions.iter().take(limit);

    println!("Recent sessions (latest first, showing up to {}):", limit);
    println!("{:<10} {:<10} {:<20} COMMAND", "ID", "STATUS", "STARTED");
    println!("{}", "-".repeat(80));

    for s in shown {
        let short_id: String = s.id.chars().take(8).collect();
        let started = s
            .start_at
            .split('T')
            .next()
            .unwrap_or(&s.start_at)
            .to_string()
            + " "
            + s.start_at
                .split('T')
                .nth(1)
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("");
        let cmd = if s.agent_cmd.len() > 45 {
            format!("{}…", &s.agent_cmd[..42])
        } else {
            s.agent_cmd.clone()
        };
        println!("{:<10} {:<10} {:<20} {}", short_id, s.status, started, cmd);
    }

    println!();
    println!("Use `il last` or `il show` to inspect the most recent session.");
    Ok(())
}

pub fn cmd_search(query: &str, limit: usize) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;
    let registry = Registry::init(&repo_root)?;
    let results = registry.search(query, limit)?;

    if results.is_empty() {
        println!("No results found for: {}", query);
        return Ok(());
    }

    println!("Search results for: {}", query);
    println!("{}", "-".repeat(80));
    println!("{:<38} {:<12} {:<28}", "SESSION ID", "STATUS", "STARTED AT");
    println!("{}", "-".repeat(80));

    for result in results {
        println!(
            "{:<38} {:<12} {:<28}",
            result.session_id, result.session.status, result.session.start_at
        );
    }

    Ok(())
}

pub fn cmd_dump(
    stream: String,
    session_id: Option<&str>,
    use_last: bool,
    output: Option<&Path>,
) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;
    let registry = Registry::init(&repo_root)?;
    let resolved_id = resolve_session_id(&registry, session_id, use_last)?;

    if registry.get_session(&resolved_id)?.is_none() {
        return Err(anyhow!("Session not found: {}", resolved_id));
    }

    let (storage_stream, pretty) = resolve_dump_request(&stream)?;
    let bytes = registry
        .read_stream_to_bytes(&resolved_id, &storage_stream)
        .map_err(|_| {
            anyhow!(
                "Stream not found: memmap_fs:sessions/{}/{}",
                resolved_id,
                storage_stream
            )
        })?;

    let output_data: Vec<u8> = if pretty {
        match stream.as_str() {
            "chat" | "conversation" => {
                let formatted = conversation::format_conversation_chat(&bytes);
                formatted.into_bytes()
            }
            _ => bytes,
        }
    } else {
        bytes
    };

    if let Some(path) = output {
        let mut file = fs::File::create(path)?;
        file.write_all(&output_data)?;
        let label = if pretty {
            if matches!(stream.as_str(), "chat" | "conversation") {
                "chat (pretty)"
            } else {
                &storage_stream
            }
        } else {
            &storage_stream
        };
        println!(
            "Wrote {} bytes from memmap_fs:sessions/{}/{} to {}",
            output_data.len(),
            resolved_id,
            label,
            path.display()
        );
    } else {
        let mut stdout = std::io::stdout();
        stdout.write_all(&output_data)?;
        stdout.flush()?;
    }

    Ok(())
}

fn resolve_dump_request(requested: &str) -> Result<(String, bool), anyhow::Error> {
    match requested {
        "chat" => Ok(("conversation".to_string(), true)),
        "conversation" => Ok(("conversation".to_string(), false)),
        "stdout" | "stdin" | "stderr" | "ring" | "events" | "normalized" | "thoughts" | "report" => {
            Ok((requested.to_string(), false))
        }
        _ => Err(anyhow!(
            "Unknown stream '{}'. Expected one of: stdout, stdin, stderr, ring, events, normalized, conversation, chat, thoughts, report",
            requested
        )),
    }
}

fn execute_non_interactive(
    repo_root: &Path,
    command: &[String],
    extra_env: &HashMap<String, String>,
) -> Result<ExecutionOutput, anyhow::Error> {
    let program = &command[0];
    let args = &command[1..];
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(repo_root);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }

    let output = cmd.output().map_err(|error| {
        anyhow!(
            "Failed to run '{}': {}. If this is GitHub Copilot CLI, install GitHub CLI and Copilot extension first.",
            program, error
        )
    })?;

    Ok(ExecutionOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stdout_bytes: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        stdin_log: String::new(),
        stdin_bytes: Vec::new(),
        exit_code: output.status.code(),
        success: output.status.success(),
        structured_events: Vec::new(),
        conversation: Vec::new(),
        normalized: Vec::new(),
        ring_buffer: Vec::new(),
    })
}

fn execute_with_pty(
    registry: &Registry,
    session_id: &str,
    repo_root: &Path,
    command: &[String],
    extra_env: &HashMap<String, String>,
) -> Result<ExecutionOutput, anyhow::Error> {
    let stdout_capture: CaptureWriter = Box::new(registry.stream_writer(session_id, "stdout"));
    let stdin_capture: CaptureWriter = Box::new(registry.stream_writer(session_id, "stdin"));

    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));

    // === 新增：live 增量结构化提取器（边跑边保存 conversation / normalized）===
    // 给 tracker 提供专用的 stream writer，这样每个用户提交 prompt 时都会立即把上一轮 agent 响应 + 本轮 user 写到 memmap_fs。
    // 退出时只需 finalize 最后一段，彻底避免 exit 时全量 VT100 replay + 提取的长时间卡顿。
    let conv_writer = Some(registry.stream_writer(session_id, "conversation"));
    let norm_writer = Some(registry.stream_writer(session_id, "normalized"));
    let tracker = Arc::new(Mutex::new(conversation::LiveConversationTracker::new(
        rows,
        cols,
        conv_writer,
        norm_writer,
    )));

    let tracker_for_stdout = Arc::clone(&tracker);
    let live_stdout_feed: Option<crate::pty::LiveChunkFeed> =
        Some(Arc::new(move |chunk: &[u8]| {
            if let Ok(mut t) = tracker_for_stdout.lock() {
                t.feed_stdout(chunk);
            }
        }));

    let tracker_for_stdin = Arc::clone(&tracker);
    let live_stdin_raw: Option<crate::pty::LiveChunkFeed> = Some(Arc::new(move |data: &[u8]| {
        if let Ok(mut t) = tracker_for_stdin.lock() {
            t.feed_stdin_raw(data);
        }
    }));

    let mut session = CompatPtySession::spawn(
        command,
        repo_root,
        extra_env,
        Some(stdout_capture),
        Some(stdin_capture),
        live_stdout_feed,
        live_stdin_raw,
    )
    .map_err(|e| {
        anyhow!(
            "Failed to spawn PTY for '{}': {}. Make sure the agent CLI exists in PATH.",
            command[0],
            e
        )
    })?;

    let status = session.wait()?;
    let (events, ring_buffer) = session.take_captures();

    // PTY 结束 → 从 memmap_fs stream 读取完整原始流（仅用于对话提取等后处理）
    let stdout_bytes = registry
        .read_stream_to_bytes(session_id, "stdout")
        .unwrap_or_default();
    let stdin_bytes = registry
        .read_stream_to_bytes(session_id, "stdin")
        .unwrap_or_default();

    let raw_stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stdout = strip_ansi_escapes::strip_str(&raw_stdout);
    let stdin_log = strip_ansi_escapes::strip_str(String::from_utf8_lossy(&stdin_bytes).as_ref());

    // 关键变更：使用 live tracker 完成最后收尾（之前的大部分内容已在用户输入时实时写出）
    // 不再调用全量 extract_with_snapshots（它会重放整个 stdout 历史做 vt100 + diff），从而让关闭瞬间几乎无感知。
    {
        let mut t = tracker.lock().expect("tracker poisoned");
        t.finalize_last_turn();
    }

    // conversation / normalized 已由 tracker 在运行时 + finalize 时增量写入对应 stream，
    // 这里返回空 vec 避免上层重复 append（保持存储内容一致）。
    let structured_events = events_to_jsonl(&events);

    Ok(ExecutionOutput {
        stdout,
        stdout_bytes,
        stderr: String::new(),
        stdin_log,
        stdin_bytes,
        exit_code: Some(status.exit_code() as i32),
        success: status.success(),
        structured_events,
        conversation: Vec::new(),
        normalized: Vec::new(),
        ring_buffer,
    })
}

fn events_to_jsonl(events: &[PtyEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|ev| serde_json::to_string(ev).ok())
        .collect()
}

fn generate_min_report(registry: &Registry, session_id: &str) -> Result<String, anyhow::Error> {
    let Some(session) = registry.get_session(session_id)? else {
        return Ok(String::new());
    };

    let mut report = String::new();
    writeln!(report, "# Session {}", session.id)?;
    writeln!(report)?;
    writeln!(report, "- Status: {}", session.status)?;
    writeln!(report, "- Start: {}", session.start_at)?;
    writeln!(
        report,
        "- End: {}",
        session.end_at.unwrap_or_else(|| "(running)".to_string())
    )?;
    writeln!(report, "- Command: {}", session.agent_cmd)?;
    writeln!(report, "- Thought events: {}", session.thought_count)?;
    writeln!(
        report,
        "- Raw stdout stream: memmap_fs:sessions/{}/stdout",
        session.id
    )?;
    writeln!(
        report,
        "- Raw stdin stream: memmap_fs:sessions/{}/stdin",
        session.id
    )?;

    Ok(report)
}
