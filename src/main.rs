use clap::{Parser, Subcommand, ValueEnum};
use std::fs;
#[cfg(feature = "zene")]
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
#[cfg(feature = "zene")]
use tokio::runtime::Runtime;
use uuid::Uuid;
#[cfg(feature = "zene")]
use zene::{AgentConfig as ZeneConfig, EventEnvelope, FileSessionStore, RunRequest as ZeneRunRequest, ZeneEngine};

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
    /// Run Zene as embedded Rust library and record full event stream
    Zene {
        /// Prompt instruction. If omitted, generated from INTENT.md
        #[arg(short, long)]
        prompt: Option<String>,
        /// Explicit Zene session id (defaults to IntentLoop session id)
        #[arg(long)]
        zene_session_id: Option<String>,
    },
}

fn main() {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run { command } => cmd_run(command),
        Commands::Copilot { prompt, mode, non_interactive, wait, args } => {
            cmd_copilot(prompt, mode, non_interactive, wait, args)
        }
        Commands::Show { session_id } => cmd_show(&session_id),
        Commands::Zene { prompt, zene_session_id } => cmd_zene(prompt, zene_session_id),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_zene(
    _prompt: Option<String>,
    _zene_session_id: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(feature = "zene"))]
    {
        return Err(
            "Zene integration is disabled in this build. Rebuild with `cargo run --features zene -- zene ...`"
                .into(),
        );
    }

    #[cfg(feature = "zene")]
    {
        return cmd_zene_enabled(_prompt, _zene_session_id);
    }
}

#[cfg(feature = "zene")]
fn cmd_zene_enabled(
    prompt: Option<String>,
    zene_session_id: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::current_dir()?;
    let registry = registry::Registry::init(&repo_root)?;
    let intent = intent::load_intent(&repo_root);

    let session_id = Uuid::now_v7().to_string();
    let session_dir = registry.session_dir_path(&session_id);
    fs::create_dir_all(&session_dir)?;

    let events_path = session_dir.join("events.jsonl");
    let raw_log_path = registry.session_log_path(&session_id);
    let report_path = registry.session_report_path(&session_id);

    let zene_sid = zene_session_id.unwrap_or_else(|| session_id.clone());
    let final_prompt = prompt.unwrap_or_else(|| intent::build_intent_prompt(&intent, "Zene"));
    let agent_cmd = format!("zene::run_envelope_stream(session_id={})", zene_sid);

    registry.create_session(
        &session_id,
        &intent.id,
        &intent.title,
        &agent_cmd,
        &repo_root,
        &raw_log_path,
    )?;

    println!("▶ Running session: {}", session_id);
    println!("Intent: {} ({})", intent.title, intent.id);
    println!("Backend: zene (embedded)");
    println!("Zene session: {}", zene_sid);

    let rt = Runtime::new()?;

    let (final_status, final_code, final_summary, error_count, first_error_message) = rt.block_on(async {
        let zene_store_dir = session_dir.join("zene_store");
        let zene_store = Arc::new(FileSessionStore::new(zene_store_dir)?);
        let config = ZeneConfig::from_env().unwrap_or_else(|_| ZeneConfig::default());
        let engine = ZeneEngine::new(config, zene_store).await?;

        let req = ZeneRunRequest {
            prompt: final_prompt,
            session_id: zene_sid.clone(),
            env_vars: None,
        };

        let mut event_rx = engine.run_envelope_stream(req).await?;

        let mut raw_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&raw_log_path)?;
        let mut events_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&events_path)?;

        writeln!(raw_file, "# session_id: {}", session_id)?;
        writeln!(raw_file, "# intent_id: {}", intent.id)?;
        writeln!(raw_file, "# intent_title: {}", intent.title)?;
        writeln!(raw_file, "# backend: zene")?;
        writeln!(raw_file, "# zene_session_id: {}", zene_sid)?;
        writeln!(raw_file)?;

        let mut seq = 1;
        let mut final_status = "succeeded".to_string();
        let mut final_code = Some(0);
        let mut final_summary = String::new();
        let mut error_count: usize = 0;
        let mut first_error_message: Option<String> = None;

        while let Some(envelope) = event_rx.recv().await {
            let envelope_json = serde_json::to_string(&envelope)?;
            writeln!(events_file, "{}", envelope_json)?;

            let line = format!(
                "[{}] {} #{}",
                envelope.ts.to_rfc3339(),
                envelope.event_type,
                envelope.seq
            );
            writeln!(raw_file, "{}", line)?;

            let payload_text = serde_json::to_string(&envelope.payload)?;
            writeln!(raw_file, "  payload: {}", payload_text)?;

            let event_line = format!("{} {}", envelope.event_type, payload_text);
            seq = registry.add_thought_events(&session_id, "zene_event", &[event_line], seq)?;

            if envelope.event_type == "Finished" {
                final_summary = extract_finished_text(&envelope).unwrap_or_else(|| "Finished".to_string());
                final_status = "succeeded".to_string();
                final_code = Some(0);
            }

            if envelope.event_type == "Error" {
                let error_message = extract_error_text(&envelope).unwrap_or_else(|| "Error".to_string());
                if first_error_message.is_none() {
                    first_error_message = Some(error_message.clone());
                }
                error_count += 1;
                final_summary = error_message;
                final_status = "failed".to_string();
                final_code = Some(1);
            }
        }

        Ok::<(String, Option<i32>, String, usize, Option<String>), Box<dyn std::error::Error>>((
            final_status,
            final_code,
            final_summary,
            error_count,
            first_error_message,
        ))
    })?;

    registry.complete_session(&session_id, &final_status, final_code)?;
    generate_min_report(&registry, &session_id)?;
    append_zene_report_tail(
        &report_path,
        &events_path,
        &final_summary,
        error_count,
        first_error_message.as_deref(),
    )?;

    println!("✓ Session saved: {}", session_id);
    println!("Events: {}", events_path.display());
    println!("Raw log: {}", raw_log_path.display());
    if error_count > 0 && final_status == "succeeded" {
        println!(
            "! Warning: run recovered after {} Error event(s). First error: {}",
            error_count,
            first_error_message.unwrap_or_else(|| "(unknown)".to_string())
        );
    }

    if final_status != "succeeded" {
        return Err(format!("Zene run failed: {}", final_summary).into());
    }

    Ok(())
}

#[cfg(feature = "zene")]
fn extract_finished_text(envelope: &EventEnvelope) -> Option<String> {
    envelope
        .payload
        .get("data")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(feature = "zene")]
fn extract_error_text(envelope: &EventEnvelope) -> Option<String> {
    if let Some(message) = envelope
        .payload
        .get("data")
        .and_then(|v| v.get("message"))
        .and_then(|v| v.as_str())
    {
        return Some(message.to_string());
    }

    envelope
        .payload
        .get("data")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(feature = "zene")]
fn append_zene_report_tail(
    report_path: &Path,
    events_path: &Path,
    summary: &str,
    error_count: usize,
    first_error_message: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut report = OpenOptions::new().append(true).open(report_path)?;
    writeln!(report)?;
    writeln!(report, "## Zene Event Stream")?;
    writeln!(report, "- Events: {}", events_path.display())?;
    writeln!(report, "- Error events during run: {}", error_count)?;
    if let Some(first_error_message) = first_error_message {
        writeln!(report, "- First error: {}", first_error_message)?;
    }
    writeln!(report, "- Final summary: {}", summary)?;
    Ok(())
}

fn cmd_run(command: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    if command.is_empty() {
        return Err("Empty command. Usage: intentloop run -- <agent-cli> [args...]".into());
    }

    let repo_root = std::env::current_dir()?;
    run_session(repo_root, command, false)
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
    })
}

fn execute_with_pty(
    repo_root: &Path,
    command: &[String],
) -> Result<ExecutionOutput, Box<dyn std::error::Error>> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 40,
        cols: 120,
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

    let output_thread = thread::spawn(move || -> std::io::Result<()> {
        let mut stdout = std::io::stdout();
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

    Ok(ExecutionOutput {
        stdout,
        stderr: String::new(),
        stdin_log,
        exit_code: i32::try_from(status.exit_code()).ok(),
        success: status.success(),
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
