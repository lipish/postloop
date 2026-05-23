use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::agent_config::AgentConfig;
use crate::conversation;
use crate::intent;
use crate::pty::{CompatPtySession, PtyEvent};
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
    /// PTY 模式下原始 stdout/stdin 的磁盘文件路径（流式捕获产物，可用于超大日志事后分析）
    pub stdout_raw_path: Option<std::path::PathBuf>,
    pub stdin_raw_path: Option<std::path::PathBuf>,
}

pub fn run_session(
    repo_root: PathBuf,
    command: Vec<String>,
    interactive: bool,
    extra_env: &HashMap<String, String>,
) -> Result<(), anyhow::Error> {
    let registry = Registry::init(&repo_root)?;
    let intent = intent::load_intent(&repo_root);

    let session_id = Uuid::now_v7().to_string();
    let session_dir = registry.session_dir_path(&session_id);
    fs::create_dir_all(&session_dir)?;
    let log_path = registry.session_log_path(&session_id);

    let agent_cmd = command.join(" ");
    registry.create_session(
        &session_id,
        &intent.id,
        &intent.title,
        &agent_cmd,
        &repo_root,
        &log_path,
    )?;

    println!("▶ Running session: {}", session_id);
    println!("Intent: {} ({})", intent.title, intent.id);
    println!("Command: {}", agent_cmd);
    if interactive {
        println!("Mode: PTY interactive");
    }

    let execution = if interactive {
        // 为 PTY 交互会话创建原始流式捕获文件（核心内存优化）
        let stdout_raw_path = session_dir.join("terminal.stdout.raw");
        let stdin_raw_path = session_dir.join("terminal.stdin.raw");
        let stdout_file = fs::File::create(&stdout_raw_path)?;
        let stdin_file = fs::File::create(&stdin_raw_path)?;

        execute_with_pty(
            &repo_root,
            &command,
            extra_env,
            stdout_raw_path,
            stdin_raw_path,
            stdout_file,
            stdin_file,
        )?
    } else {
        execute_non_interactive(&repo_root, &command, extra_env)?
    };

    let mut log_file = fs::File::create(&log_path)?;
    writeln!(log_file, "# session_id: {}", session_id)?;
    writeln!(log_file, "# intent_id: {}", intent.id)?;
    writeln!(log_file, "# intent_title: {}", intent.title)?;
    writeln!(log_file, "# command: {}", agent_cmd)?;
    writeln!(log_file, "# mode: {}", if interactive { "pty" } else { "non-interactive" })?;
    writeln!(log_file)?;
    if !execution.stdin_bytes.is_empty() {
        writeln!(log_file, "[stdin]")?;
        log_file.write_all(&execution.stdin_bytes)?;
        writeln!(log_file)?;
    }
    writeln!(log_file, "[stdout]")?;
    log_file.write_all(&execution.stdout_bytes)?;
    writeln!(log_file)?;
    writeln!(log_file, "[stderr]")?;
    writeln!(log_file, "{}", execution.stderr)?;

    if !execution.structured_events.is_empty() {
        let events_path = session_dir.join("events.jsonl");
        let mut events_file = fs::File::create(&events_path)?;
        for ev in &execution.structured_events {
            writeln!(events_file, "{}", ev)?;
        }
        println!("Structured events: {}", events_path.display());
    }

    if !execution.normalized.is_empty() {
        let norm_path = session_dir.join("terminal.normalized.jsonl");
        let mut norm_file = fs::File::create(&norm_path)?;
        for line in &execution.normalized {
            writeln!(norm_file, "{}", line)?;
        }
        println!("VT100 normalized: {}", norm_path.display());
    }

    if !execution.conversation.is_empty() {
        let conv_path = session_dir.join("conversation.jsonl");
        let mut conv_file = fs::File::create(&conv_path)?;
        for line in &execution.conversation {
            writeln!(conv_file, "{}", line)?;
        }
        println!("Conversation log: {}", conv_path.display());
    }

    if !execution.ring_buffer.is_empty() {
        let ring_path = session_dir.join("terminal.ring.bin");
        fs::write(&ring_path, &execution.ring_buffer)?;
        println!("Ring buffer: {}", ring_path.display());
    }

    if let Some(p) = &execution.stdout_raw_path {
        println!("Raw stdout stream: {}", p.display());
    }
    if let Some(p) = &execution.stdin_raw_path {
        println!("Raw stdin stream: {}", p.display());
    }

    let stdout_lines: Vec<String> = execution.stdout.lines().map(str::to_string).collect();
    let stderr_lines: Vec<String> = execution.stderr.lines().map(str::to_string).collect();
    let stdin_lines: Vec<String> = execution.stdin_log.lines().map(str::to_string).collect();

    let mut seq = 1i64;
    let mut thought_count = 0i64;
    if !stdin_lines.is_empty() {
        let (next_seq, added) = registry.add_thought_events(&session_id, "stdin", &stdin_lines, seq)?;
        seq = next_seq;
        thought_count += added;
    }
    let (next_seq, added) = registry.add_thought_events(&session_id, "stdout", &stdout_lines, seq)?;
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

    generate_min_report(&registry, &session_id)?;

    println!("✓ Session saved: {}", session_id);
    println!("Raw log: {}", log_path.display());

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

pub fn cmd_run(
    agent: Option<String>,
    command: Vec<String>,
    non_interactive: bool,
) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;
    let config = AgentConfig::load(&repo_root);
    let mut extra_env = HashMap::new();

    let final_command = if let Some(agent_name) = agent {
        if let Some(profile_cmd) = config.resolve_command(&agent_name, &command, None, Some(&repo_root)) {
            println!("▶ Launching agent '{}' in {}", agent_name, repo_root.display());
            println!("   Command: {} {}", profile_cmd[0], profile_cmd[1..].join(" "));

            extra_env = config.build_env(&agent_name);
            config.apply_shell_setup(&agent_name, profile_cmd)
        } else {
            if command.is_empty() {
                return Err(anyhow!("Unknown agent '{}'. Please define it in .intent/agents.toml", agent_name));
            }
            command
        }
    } else {
        if command.is_empty() {
            return Err(anyhow!("Empty command. Usage: intent run --agent <name>  or  intent run -- <agent-cli> [args...]"));
        }
        command
    };

    run_session(repo_root, final_command, !non_interactive, &extra_env)
}

pub fn cmd_show(session_id: &str) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;
    let registry = Registry::init(&repo_root)?;
    let Some(session) = registry.get_session(session_id)? else {
        return Err(anyhow!("Session not found: {}", session_id));
    };

    println!("Session: {}", session.id);
    println!("Intent: {} ({})", session.intent_title, session.intent_id);
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
    println!("Raw log: {}", session.log_path);

    let report_path = registry.session_report_path(session_id);
    if report_path.exists() {
        println!("Report: {}", report_path.display());
    }

    let ring_path = registry.session_dir_path(session_id).join("terminal.ring.bin");
    if ring_path.exists() {
        let size = fs::metadata(&ring_path)?.len();
        println!("Ring buffer: {} ({} bytes)", ring_path.display(), size);
    }

    Ok(())
}

pub fn cmd_list() -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;
    let registry = Registry::init(&repo_root)?;
    let sessions = registry.list_sessions()?;

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!("{:<38} {:<24} {:<12} {:<28}", "SESSION ID", "INTENT", "STATUS", "STARTED AT");
    println!("{}", "-".repeat(105));
    for s in sessions {
        let intent_title = if s.intent_title.len() > 24 {
            format!("{}...", &s.intent_title[..21])
        } else {
            s.intent_title.clone()
        };
        println!("{:<38} {:<24} {:<12} {:<28}", s.id, intent_title, s.status, s.start_at);
    }
    Ok(())
}

pub fn cmd_attach(session_id: &str) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;
    let registry = Registry::init(&repo_root)?;
    let Some(session) = registry.get_session(session_id)? else {
        return Err(anyhow!("Session not found: {}", session_id));
    };

    if session.status == "running" {
        return Err(anyhow!("Live attach is not yet supported. Wait for the session to finish."));
    }

    let ring_path = registry.session_dir_path(session_id).join("terminal.ring.bin");
    if !ring_path.exists() {
        return Err(anyhow!(
            "No ring buffer for session {}. Re-run with PTY interactive mode to capture one.",
            session_id
        ));
    }

    let ring = fs::read(&ring_path)?;
    let preview = String::from_utf8_lossy(&ring);
    let tail: String = preview.chars().rev().take(2_048).collect::<String>().chars().rev().collect();

    println!("Session: {} ({})", session.id, session.status);
    println!("Ring buffer: {} bytes", ring.len());
    println!("--- tail preview ---");
    print!("{tail}");
    if !tail.ends_with('\n') {
        println!();
    }

    Ok(())
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
        stdout_raw_path: None,
        stdin_raw_path: None,
    })
}

fn execute_with_pty(
    repo_root: &Path,
    command: &[String],
    extra_env: &HashMap<String, String>,
    stdout_raw_path: std::path::PathBuf,
    stdin_raw_path: std::path::PathBuf,
    stdout_capture: std::fs::File,
    stdin_capture: std::fs::File,
) -> Result<ExecutionOutput, anyhow::Error> {
    let mut session = CompatPtySession::spawn(
        command,
        repo_root,
        extra_env,
        Some(stdout_capture),
        Some(stdin_capture),
    )
    .map_err(|e| {
        anyhow!(
            "Failed to spawn PTY for '{}': {}. Make sure the agent CLI exists in PATH.",
            command[0], e
        )
    })?;

    let status = session.wait()?;
    let (events, ring_buffer) = session.take_captures();

    // PTY 结束 → 从磁盘文件读取完整原始流（此时才短暂占用内存，用于对话提取等后处理）
    let stdout_bytes = std::fs::read(&stdout_raw_path).unwrap_or_default();
    let stdin_bytes = std::fs::read(&stdin_raw_path).unwrap_or_default();

    let raw_stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stdout = strip_ansi_escapes::strip_str(&raw_stdout);
    let stdin_log = strip_ansi_escapes::strip_str(String::from_utf8_lossy(&stdin_bytes).as_ref());

    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));

    let (snapshots, turns) = conversation::extract_with_snapshots(&stdout_bytes, &stdin_bytes, rows, cols);
    let conversation = conversation::turns_to_jsonl(&turns);
    let normalized = conversation::snapshots_to_jsonl(&snapshots);
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
        conversation,
        normalized,
        ring_buffer,
        stdout_raw_path: Some(stdout_raw_path),
        stdin_raw_path: Some(stdin_raw_path),
    })
}

fn events_to_jsonl(events: &[PtyEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|ev| serde_json::to_string(ev).ok())
        .collect()
}

fn generate_min_report(registry: &Registry, session_id: &str) -> Result<(), anyhow::Error> {
    let Some(session) = registry.get_session(session_id)? else {
        return Ok(());
    };

    let report_path = registry.session_report_path(session_id);
    let mut report_file = fs::File::create(report_path)?;

    writeln!(report_file, "# Session {}", session.id)?;
    writeln!(report_file)?;
    writeln!(report_file, "- Intent: {} ({})", session.intent_title, session.intent_id)?;
    writeln!(report_file, "- Status: {}", session.status)?;
    writeln!(report_file, "- Start: {}", session.start_at)?;
    writeln!(
        report_file,
        "- End: {}",
        session.end_at.unwrap_or_else(|| "(running)".to_string())
    )?;
    writeln!(report_file, "- Command: {}", session.agent_cmd)?;
    writeln!(report_file, "- Thought events: {}", session.thought_count)?;
    writeln!(report_file, "- Raw log: {}", session.log_path)?;

    Ok(())
}
