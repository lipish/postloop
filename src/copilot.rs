use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use crate::intent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopilotMode {
    Auto,
    Copilot,
    AgentTask,
}

static COPILOT_CLI_AVAILABLE: OnceLock<bool> = OnceLock::new();

pub fn build_gh_agent_command(
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

pub fn resolve_copilot_mode(repo_root: &Path, mode: CopilotMode, first_arg: Option<&str>) -> CopilotMode {
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

            if copilot_subcommand_available(repo_root) {
                return CopilotMode::Copilot;
            }

            CopilotMode::AgentTask
        }
    }
}

pub fn mode_label(mode: CopilotMode) -> &'static str {
    match mode {
        CopilotMode::Copilot => "gh copilot",
        CopilotMode::AgentTask => "gh agent-task",
        CopilotMode::Auto => "auto",
    }
}

fn copilot_subcommand_available(repo_root: &Path) -> bool {
    *COPILOT_CLI_AVAILABLE.get_or_init(|| supports_subcommand(repo_root, &["copilot", "--help"]))
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
