use clap::{Parser, Subcommand, ValueEnum};
use intent::copilot::{self, CopilotMode};
use intent::session;

#[derive(Parser)]
#[command(name = "intent")]
#[command(about = "Intent - interactive agent session recorder", long_about = None)]
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
    /// Run agent command and record a session
    Run {
        /// Agent name defined in .intent/agents.toml, e.g. cursor, claude, copilot
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
        #[arg(long, value_enum, default_value_t = CopilotModeCli::Auto)]
        mode: CopilotModeCli,
        /// Disable PTY mode and execute in non-interactive capture mode
        #[arg(long)]
        non_interactive: bool,
        /// Wait for final result when backend supports it (agent-task -> --follow)
        #[arg(long)]
        wait: bool,
        /// Raw args for `gh copilot`, e.g. `intent copilot -- suggest "fix auth bug"`
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
    Attach {
        session_id: String,
    },
}

fn main() {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run { agent, command, non_interactive } => session::cmd_run(agent, command, non_interactive),
        Commands::Copilot { prompt, mode, non_interactive, wait, args } => {
            cmd_copilot(prompt, mode.into(), non_interactive, wait, args)
        }
        Commands::Show { session_id } => session::cmd_show(&session_id),
        Commands::List => session::cmd_list(),
        Commands::Attach { session_id } => session::cmd_attach(&session_id),
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
    let intent = intent::load_intent(&repo_root);

    let selected_mode = copilot::resolve_copilot_mode(&repo_root, mode, args.first().map(String::as_str));
    let command = copilot::build_gh_agent_command(selected_mode, prompt, args, &intent, wait);

    println!("Copilot backend: {}", copilot::mode_label(selected_mode));
    if wait {
        println!("Wait mode: enabled");
    }

    session::run_session(repo_root, command, !non_interactive, &Default::default())
}
