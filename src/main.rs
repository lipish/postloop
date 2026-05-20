use clap::{Parser, Subcommand, ValueEnum};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use uuid::Uuid;

mod agent_config;
mod conversation;
mod intent;
mod pty;
mod registry;

#[derive(Parser)]
#[command(name = "intent")]
#[command(about = "Intent - interactive agent session recorder", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CopilotMode {
    Auto,
    Copilot,
    AgentTask,
}

#[derive(Subcommand)]
enum Commands {
    /// Run agent command and record a session
    Run {
        /// Agent name defined in .intentloop/agents.toml, e.g. cursor, claude, copilot
        #[arg(long)]
        agent: Option<String>,
        /// Optional extra args or full command. When --agent is used without this, launches the agent interactively.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        /// Disable PTY mode and execute in non-interactive capture mode
        #[arg(long)]
        non_interactive: bool,
    },
    /// Run GitHub Copilot CLI in an IntentLoop session
    Copilot {
        /// Prompt passed to `gh copilot suggest`; defaults to INTENT.md-derived prompt
        #[arg(short, long)]
        prompt: Option<String>,
        /// Backend mode: auto (detect), copilot (`gh copilot`), or agent-task (`gh agent-task`)
        #[arg(long, value_enum, default_value_t = CopilotMode::Auto)]
        mode: CopilotMode,
        /// Disable PTY mode and execute in non-interactive capture mode
        #[arg(long)]
        non_interactive: bool,
        /// Wait for final result when backend supports it (agent-task -> --follow)
        #[arg(long)]
        wait: bool,
        /// Raw args for `gh copilot`, e.g. `intentloop copilot -- suggest "fix auth bug"`
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Show a recorded session
    Show {
        /// Session ID
        session_id: String,
    },
    /// List recent sessions
    List,
    /// Attach to a running session (future)
    Attach {
        session_id: String,
    },
}

fn main() {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run { agent, command, non_interactive } => cmd_run(agent, command, non_interactive),
        Commands::Copilot { prompt, mode, non_interactive, wait, args } => {
            cmd_copilot(prompt, mode, non_interactive, wait, args)
        }
        Commands::Show { session_id } => cmd_show(&session_id),
        Commands::List => cmd_list(),
        Commands::Attach { session_id } => cmd_attach(&session_id),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_run(agent: Option<String>, command: Vec<String>, non_interactive: bool) -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::current_dir()?;
    let config = agent_config::AgentConfig::load(&repo_root);

    let final_command = if let Some(agent_name) = agent {
        if let Some(mut profile_cmd) = config.resolve_command(&agent_name, &command, None, Some(&repo_root)) {
            println!("▶ Launching agent '{}' in {}", agent_name, repo_root.display());
            println!("   Command: {} {}", profile_cmd[0], profile_cmd[1..].join(" "));

            profile_cmd
        } else {
            if command.is_empty() {
                return Err(format!("Unknown agent '{}'. Please define it in .intent/agents.toml", agent_name).into());
            }
            command
        }
    } else {
        if command.is_empty() {
            return Err("Empty command. Usage: intent run --agent <name>  or  intent run -- <agent-cli> [args...]".into());
        }
        command
    };

    run_session(repo_root, final_command, !non_interactive)
}

fn cmd_copilot(
    prompt: Option<String>,
    mode: CopilotMode,
    non_interactive: bool,
    wait: bool,
    args: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::current_dir()?;
    let intent = intent::load_intent(&repo_root);

    let selected_mode = resolve_copilot_mode(&repo_root, mode, args.first().map(String::as_str));

    let command = build_gh_agent_command(selected_mode, prompt, args, &intent, wait);

    let mode_label = match selected_mode {
        CopilotMode::Copilot => "gh copilot",
        CopilotMode::AgentTask => "gh agent-task",
        CopilotMode::Auto => "auto",
    };
    println!("Copilot backend: {}", mode_label);
    if wait {
        println!("Wait mode: enabled");
    }

    run_session(repo_root, command, !non_interactive)
}

fn build_gh_agent_command(
    selected_mode: CopilotMode,
    prompt: Option<String>,
    mut args: Vec<String>,
    intent_info: &intent::IntentInfo,
    wait: bool,
) -> Vec<String> {
    let mut cmd = vec!["gh".to_string()];

    match selected_mode {
        CopilotMode::Copilot => {
            cmd.push("copilot".to_string());

            if wait {
                eprintln!("Warning: --wait is currently only supported for gh agent-task backend.");
            }

            if args.is_empty() {
                let final_prompt = prompt.unwrap_or_else(|| intent::build_copilot_prompt(intent_info));
                cmd.push("suggest".to_string());
                cmd.push(final_prompt);
            } else {
                cmd.extend(args);
            }
        }
        CopilotMode::AgentTask | CopilotMode::Auto => {
            cmd.push("agent-task".to_string());

            if args.is_empty() {
                let final_prompt = prompt.unwrap_or_else(|| intent::build_copilot_prompt(intent_info));
                cmd.push("create".to_string());
                cmd.push(final_prompt);
                if wait {
                    cmd.push("--follow".to_string());
                }
            } else {
                if wait {
                    let has_create = args.iter().any(|arg| arg == "create");
                    let has_follow = args.iter().any(|arg| arg == "--follow");
                    if has_create && !has_follow {
                        args.push("--follow".to_string());
                    }
                }
                cmd.extend(args);
            }
        }
    }

    cmd
}

fn resolve_copilot_mode(repo_root: &Path, mode: CopilotMode, first_arg: Option<&str>) -> CopilotMode {
    match mode {
        CopilotMode::Copilot => CopilotMode::Copilot,
        CopilotMode::AgentTask => CopilotMode::AgentTask,
        CopilotMode::Auto => {
            if let Some(value) = first_arg {
                if ["suggest", "explain"].contains(&value) {
                    return CopilotMode::Copilot;
                }

                if ["create", "list", "view"].contains(&value) {
                    return CopilotMode::AgentTask;
                }
            }

            if supports_subcommand(repo_root, &["copilot", "--help"]) {
                return CopilotMode::Copilot;
            }

            CopilotMode::AgentTask
        }
    }
}

fn supports_subcommand(repo_root: &Path, args: &[&str]) -> bool {
    let Ok(output) = Command::new("gh")
        .args(args)
        .current_dir(repo_root)
        .output()
    else {
        return false;
    };

    output.status.success()
}

fn run_session(
    repo_root: PathBuf,
    command: Vec<String>,
    interactive: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = registry::Registry::init(&repo_root)?;
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
        execute_with_pty(&repo_root, &command)?
    } else {
        execute_non_interactive(&repo_root, &command)?
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

    // 干净对话记录（vt100 屏幕 diff + stdin 行编辑）
    if !execution.conversation.is_empty() {
        let conv_path = session_dir.join("conversation.jsonl");
        let mut conv_file = fs::File::create(&conv_path)?;
        for line in &execution.conversation {
            writeln!(conv_file, "{}", line)?;
        }
        println!("Conversation log: {}", conv_path.display());
    }

    let stdout_lines: Vec<String> = execution.stdout.lines().map(|line| line.to_string()).collect();
    let stderr_lines: Vec<String> = execution.stderr.lines().map(|line| line.to_string()).collect();
    let stdin_lines: Vec<String> = execution.stdin_log.lines().map(|line| line.to_string()).collect();

    let mut seq = 1;
    if !stdin_lines.is_empty() {
        seq = registry.add_thought_events(&session_id, "stdin", &stdin_lines, seq)?;
    }
    seq = registry.add_thought_events(&session_id, "stdout", &stdout_lines, seq)?;
    registry.add_thought_events(&session_id, "stderr", &stderr_lines, seq)?;

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
        return Err(format!(
            "Agent command exited with status {}",
            execution
                .exit_code
                .map(|v| v.to_string())
                .unwrap_or_else(|| "terminated by signal".to_string())
        )
        .into());
    }

    Ok(())
}

struct ExecutionOutput {
    stdout: String,
    stdout_bytes: Vec<u8>,
    stderr: String,
    stdin_log: String,
    stdin_bytes: Vec<u8>,
    exit_code: Option<i32>,
    success: bool,
    structured_events: Vec<String>,
    conversation: Vec<String>,
    normalized: Vec<String>,
}

fn execute_non_interactive(
    repo_root: &Path,
    command: &[String],
) -> Result<ExecutionOutput, Box<dyn std::error::Error>> {
    let program = &command[0];
    let args = &command[1..];
    let output = Command::new(program).args(args).current_dir(repo_root).output().map_err(|error| {
        format!(
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
    })
}

/// 使用 100% 兼容 PTY 实现执行（支持完整交互、raw mode、动态 resize）
fn execute_with_pty(
    repo_root: &Path,
    command: &[String],
) -> Result<ExecutionOutput, Box<dyn std::error::Error>> {
    use crate::pty::CompatPtySession;

    let mut session = CompatPtySession::spawn(command, repo_root).map_err(|e| {
        format!(
            "Failed to spawn PTY for '{}': {}. Make sure the agent CLI exists in PATH.",
            command[0], e
        )
    })?;

    // 阻塞等待子进程结束（此时 raw mode 已启用，输入输出完全透传）
    let status: portable_pty::ExitStatus = session.wait()?;

    // 收集记录
    let stdout_bytes = session.stdout.lock().unwrap().clone();
    let stdin_bytes = session.stdin.lock().unwrap().clone();
    let events = session.events.lock().unwrap().clone();

    let raw_stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stdout = strip_ansi_escapes::strip_str(&raw_stdout);
    let stdin_log = strip_ansi_escapes::strip_str(&String::from_utf8_lossy(&stdin_bytes));

    let (pty_rows, pty_cols) = crossterm::terminal::size().unwrap_or((40, 120));

    // vt100 回放：从原始 PTY 字节流提取屏幕快照与对话
    let (snapshots, turns) =
        conversation::extract_with_snapshots(&stdout_bytes, &stdin_bytes, pty_rows, pty_cols);
    let conversation = conversation::turns_to_jsonl(&turns);
    let normalized: Vec<String> = conversation::snapshots_to_jsonl(&snapshots);

    let structured_events: Vec<String> = events
        .iter()
        .map(|ev| {
            if let Some(b) = ev.bytes {
                format!(r#"{{"type":"{}","bytes":{},"ts":"{}"}}"#, ev.kind, b, ev.ts)
            } else {
                format!(r#"{{"type":"{}","ts":"{}"}}"#, ev.kind, ev.ts)
            }
        })
        .collect();

    let success = status.success();
    let exit_code = if success { Some(0) } else { Some(1) };

    Ok(ExecutionOutput {
        stdout,
        stdout_bytes,
        stderr: String::new(),
        stdin_log,
        stdin_bytes,
        exit_code,
        success,
        structured_events,
        conversation,
        normalized,
    })
}

fn cmd_show(session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::current_dir()?;
    let registry = registry::Registry::init(&repo_root)?;
    let Some(session) = registry.get_session(session_id)? else {
        return Err(format!("Session not found: {}", session_id).into());
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

    Ok(())
}

fn cmd_list() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::current_dir()?;
    let registry = registry::Registry::init(&repo_root)?;
    // 简化：实际应查询最近 N 条
    println!("Use `intentloop show <id>` to inspect sessions. Full list coming soon.");
    Ok(())
}

fn cmd_attach(session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Attach to session {} (not yet implemented, will support ring buffer replay)", session_id);
    Ok(())
}

fn generate_min_report(registry: &registry::Registry, session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
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
