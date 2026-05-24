use clap::{Parser, Subcommand, ValueEnum};
use intent::copilot::{self, CopilotMode};
use intent::session;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "il")]
#[command(version)]
#[command(about = "il - record full AI agent sessions with one command", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CopilotModeCli {
    Auto,
    Copilot,
    AgentTask,
}

impl From<CopilotModeCli> for CopilotMode {
    fn from(value: CopilotModeCli) -> Self {
        match value {
            CopilotModeCli::Auto => CopilotMode::Auto,
            CopilotModeCli::Copilot => CopilotMode::Copilot,
            CopilotModeCli::AgentTask => CopilotMode::AgentTask,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Run any agent or command and record the full interactive session.
    ///
    /// Works out of the box for anything already in your PATH (Cursor, Claude,
    /// Kimi, etc.). Use .intent/agents.toml only when you need custom shell
    /// activation or fixed arguments.
    ///
    ///   il run cursor
    ///   il run claude
    ///   il run kimi
    ///   il run echo "hello"
    Run {
        /// Agent name or executable in PATH
        agent: String,
        /// Extra arguments (only used in direct/PATH mode)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run GitHub Copilot CLI in an IntentLoop session
    Copilot {
        /// Prompt passed to `gh copilot suggest`
        #[arg(short, long)]
        prompt: Option<String>,
        /// Backend mode: auto (detect), copilot (`gh copilot`), or agent-task (`gh agent-task`)
        #[arg(long, value_enum, default_value_t = CopilotModeCli::Auto)]
        mode: CopilotModeCli,
        /// Disable PTY mode and execute in non-interactive capture mode
        #[arg(long)]
        non_interactive: bool,
        /// Wait for final result when backend supports it (agent-task -> --follow)
        #[arg(long)]
        wait: bool,
        /// Raw args for `gh copilot`, e.g. `il copilot -- suggest "fix auth bug"`
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
    /// Replay the saved ring buffer tail for a completed session
    Attach { session_id: String },
    /// Search across all session conversations
    Search {
        /// Search query
        query: String,
        /// Maximum number of results
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Dump a stored memmap_fs stream for a session
    Dump {
        /// Session ID
        session_id: String,
        /// Stream name: stdout, stdin, stderr, ring, events, normalized, conversation, thoughts, report
        stream: String,
        /// Write output to a file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run { agent, args } => session::cmd_run(agent, args),
        Commands::Copilot {
            prompt,
            mode,
            non_interactive,
            wait,
            args,
        } => cmd_copilot(prompt, mode.into(), non_interactive, wait, args),
        Commands::Show { session_id } => session::cmd_show(&session_id),
        Commands::List => session::cmd_list(),
        Commands::Attach { session_id } => session::cmd_attach(&session_id),
        Commands::Search { query, limit } => session::cmd_search(&query, limit),
        Commands::Dump {
            session_id,
            stream,
            output,
        } => session::cmd_dump(&session_id, &stream, output.as_deref()),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_copilot(
    prompt: Option<String>,
    mode: CopilotMode,
    non_interactive: bool,
    wait: bool,
    args: Vec<String>,
) -> Result<(), anyhow::Error> {
    let repo_root = std::env::current_dir()?;

    let selected_mode =
        copilot::resolve_copilot_mode(&repo_root, mode, args.first().map(String::as_str));
    let command = copilot::build_gh_agent_command(selected_mode, prompt, args, wait);

    println!("Copilot backend: {}", copilot::mode_label(selected_mode));
    if wait {
        println!("Wait mode: enabled");
    }

    session::run_session(repo_root, command, !non_interactive, &Default::default())
}
