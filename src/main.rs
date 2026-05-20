use clap::{Parser, Subcommand, ValueEnum};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use uuid::Uuid;

mod intent;
mod registry;

#[derive(Parser)]
#[command(name = "intentloop")]
#[command(about = "IntentLoop Lite - local agent session recorder", long_about = None)]
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
        /// Agent command and arguments, e.g. `intentloop run -- claude code`
        #[arg(required = true, trailing_var_arg = true)]
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
}

fn main() {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run { command, non_interactive } => cmd_run(command, non_interactive),
        Commands::Copilot { prompt, mode, non_interactive, wait, args } => {
            cmd_copilot(prompt, mode, non_interactive, wait, args)
        }
        Commands::Show { session_id } => cmd_show(&session_id),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_run(command: Vec<String>, non_interactive: bool) -> Result<(), Box<dyn std::error::Error>> {
    if command.is_empty() {
        return Err("Empty command. Usage: intentloop run -- <agent-cli> [args...]".into());
    }

    let repo_root = std::env::current_dir()?;
    run_session(repo_root, command, !non_interactive)
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
    if !execution.stdin_log.is_empty() {
        writeln!(log_file, "[stdin]")?;
        writeln!(log_file, "{}", execution.stdin_log)?;
    }
    writeln!(log_file, "[stdout]")?;
    writeln!(log_file, "{}", execution.stdout)?;
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
    stderr: String,
    stdin_log: String,
    exit_code: Option<i32>,
    success: bool,
    structured_events: Vec<String>,
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
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        stdin_log: String::new(),
        exit_code: output.status.code(),
        success: output.status.success(),
        structured_events: Vec::new(),
    })
}

fn execute_with_pty(
    repo_root: &Path,
    command: &[String],
) -> Result<ExecutionOutput, Box<dyn std::error::Error>> {
    let pty_system = native_pty_system();
    let rows: u16 = 40;
    let cols: u16 = 120;
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new(&command[0]);
    cmd.args(&command[1..]);
    cmd.cwd(repo_root);

    let mut child = pair.slave.spawn_command(cmd).map_err(|error| {
        format!(
            "Failed to run '{}': {}. If this is GitHub Copilot CLI, install GitHub CLI and Copilot extension first.",
            command[0], error
        )
    })?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;

    let output_capture = Arc::new(Mutex::new(Vec::<u8>::new()));
    let output_capture_for_thread = Arc::clone(&output_capture);

    let structured_events = Arc::new(Mutex::new(Vec::<String>::new()));
    let structured_events_for_thread = Arc::clone(&structured_events);

    let output_thread = thread::spawn(move || -> std::io::Result<()> {
        let mut stdout = std::io::stdout();
        let mut parser = vt100::Parser::new(rows, cols, 0);
        let mut last_text = String::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read_bytes = reader.read(&mut buffer)?;
            if read_bytes == 0 {
                break;
            }
            stdout.write_all(&buffer[..read_bytes])?;
            stdout.flush()?;
            output_capture_for_thread
                .lock()
                .expect("capture lock poisoned")
                .extend_from_slice(&buffer[..read_bytes]);

            // Always emit a raw output event for every chunk (guarantees events.jsonl for all commands)
            let chunk_str = String::from_utf8_lossy(&buffer[..read_bytes]);
            let output_event = format!(
                r#"{{"type":"PtyOutput","data":{},"bytes":{},"ts":"{}"}}"#,
                serde_json::to_string(&chunk_str).unwrap_or_default(),
                read_bytes,
                chrono::Utc::now().to_rfc3339()
            );
            structured_events_for_thread
                .lock()
                .expect("events lock poisoned")
                .push(output_event);

            // Additionally record ScreenUpdate when visible screen content changes (for TUI / full redraws)
            parser.process(&buffer[..read_bytes]);
            let screen = parser.screen();
            let text = screen.contents();
            if text != last_text {
                let pos = screen.cursor_position();
                let event = format!(
                    r#"{{"type":"ScreenUpdate","text":{},"cursor_row":{},"cursor_col":{},"ts":"{}"}}"#,
                    serde_json::to_string(&text).unwrap_or_default(),
                    pos.0,
                    pos.1,
                    chrono::Utc::now().to_rfc3339()
                );
                structured_events_for_thread
                    .lock()
                    .expect("events lock poisoned")
                    .push(event);
                last_text = text;
            }
        }
        Ok(())
    });

    let input_capture = Arc::new(Mutex::new(Vec::<u8>::new()));
    let input_capture_for_thread = Arc::clone(&input_capture);
    thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0_u8; 1024];
        loop {
            let Ok(read_bytes) = stdin.read(&mut buffer) else {
                break;
            };
            if read_bytes == 0 {
                break;
            }

            if writer.write_all(&buffer[..read_bytes]).is_err() {
                break;
            }

            input_capture_for_thread
                .lock()
                .expect("stdin capture lock poisoned")
                .extend_from_slice(&buffer[..read_bytes]);
        }
    });

    let status = child.wait()?;
    drop(pair.master);

    if let Ok(result) = output_thread.join() {
        if let Err(error) = result {
            eprintln!("Warning: failed to capture PTY output: {}", error);
        }
    }

    let stdout = String::from_utf8_lossy(&output_capture.lock().expect("capture lock poisoned")).to_string();
    let stdin_log = String::from_utf8_lossy(&input_capture.lock().expect("stdin capture lock poisoned")).to_string();
    let events = structured_events.lock().expect("events lock poisoned").clone();

    Ok(ExecutionOutput {
        stdout,
        stderr: String::new(),
        stdin_log,
        exit_code: i32::try_from(status.exit_code()).ok(),
        success: status.success(),
        structured_events: events,
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
