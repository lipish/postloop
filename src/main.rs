use clap::{Parser, Subcommand};
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

#[derive(Subcommand)]
enum Commands {
    /// Run any agent or command and record the full interactive session.
    ///
    /// Launches the command exactly as it would run in your current shell
    /// (full environment, PATH, activated conda/venv/direnv/asdf/nvm, login state, etc.).
    /// IntentLoop does not read any agents.toml or manage launch configuration.
    ///
    ///   il run cursor
    ///   il run claude
    ///   il run kimi
    ///   il run echo "hello"
    Run {
        /// Agent name or executable in PATH (executed exactly as in your current shell)
        agent: String,
        /// Extra arguments passed verbatim to the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Show a recorded session (defaults to the most recent if no ID given)
    Show {
        /// Session ID (optional; defaults to latest session)
        session_id: Option<String>,
        /// Explicitly operate on the most recent session
        #[arg(long, conflicts_with = "session_id")]
        last: bool,
    },
    /// Show the most recent session (shorthand for `il show` or `il show --last`)
    Last,
    /// List recent sessions (latest first)
    List {
        /// Maximum number of sessions to show (default 10)
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Search across all session conversations
    Search {
        /// Search query
        query: String,
        /// Maximum number of results
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Dump a stored memmap_fs stream for a session (defaults to latest)
    Dump {
        /// Stream to dump.
        /// Use 'chat' for a human-readable conversation view (recommended).
        /// Raw JSONL is available via 'conversation'.
        stream: String,
        /// Session ID (optional; defaults to latest session)
        session_id: Option<String>,
        /// Explicitly operate on the most recent session
        #[arg(long, conflicts_with = "session_id")]
        last: bool,
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
        Commands::Show { session_id, last } => session::cmd_show(session_id.as_deref(), last),
        Commands::Last => session::cmd_show(None, true),
        Commands::List { limit } => session::cmd_list(limit),
        Commands::Search { query, limit } => session::cmd_search(&query, limit),
        Commands::Dump {
            stream,
            session_id,
            last,
            output,
        } => session::cmd_dump(stream, session_id.as_deref(), last, output.as_deref()),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
