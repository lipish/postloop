use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentProfile {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub shell_setup: Option<String>,
    #[serde(default)]
    pub env_whitelist: Vec<String>,
    #[serde(default)]
    pub prompt_template: Option<String>,
    #[serde(default)]
    pub supports_tmux: bool,
    /// 为 true 时在命令末尾追加工作区路径（也可用 args 里的 "{cwd}" 占位符）
    #[serde(default)]
    pub pass_cwd: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentConfig {
    #[serde(default)]
    pub agents: HashMap<String, AgentProfile>,
}

impl AgentConfig {
    /// Load agents.toml with the following priority:
    /// 1. .intent/agents.toml in current directory or any parent directory (walk up)
    /// 2. ~/.intent/agents.toml (user global config)
    /// 3. Legacy: .intentloop/agents.toml (for backward compatibility)
    pub fn load(start_dir: &Path) -> Self {
        // 1. Walk up from start_dir to find .intent/agents.toml
        let mut dir = start_dir.to_path_buf();
        loop {
            let candidate = dir.join(".intent").join("agents.toml");
            if let Ok(content) = fs::read_to_string(&candidate) {
                if let Ok(cfg) = toml::from_str::<AgentConfig>(&content) {
                    return cfg;
                }
            }
            // legacy fallback in same dir
            let legacy = dir.join(".intentloop").join("agents.toml");
            if let Ok(content) = fs::read_to_string(&legacy) {
                if let Ok(cfg) = toml::from_str::<AgentConfig>(&content) {
                    return cfg;
                }
            }
            if !dir.pop() {
                break;
            }
        }

        // 2. User global config: ~/.intent/agents.toml
        if let Ok(home) = std::env::var("HOME") {
            let global = std::path::Path::new(&home).join(".intent").join("agents.toml");
            if let Ok(content) = fs::read_to_string(&global) {
                if let Ok(cfg) = toml::from_str::<AgentConfig>(&content) {
                    return cfg;
                }
            }
        }

        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&AgentProfile> {
        self.agents.get(name)
    }

    /// Build the command vector for the given agent.
    /// If extra_args is empty, just use profile.command + profile.args (interactive mode).
    /// If prompt is provided, append it using prompt_template if present.
    pub fn resolve_command(
        &self,
        name: &str,
        extra_args: &[String],
        prompt: Option<&str>,
        cwd: Option<&Path>,
    ) -> Option<Vec<String>> {
        let profile = self.get(name)?;
        let mut cmd = vec![profile.command.clone()];
        cmd.extend(substitute_cwd(&profile.args, cwd));

        if !extra_args.is_empty() {
            cmd.extend(extra_args.to_vec());
        }

        if profile.pass_cwd {
            if let Some(dir) = cwd {
                cmd.push(dir.to_string_lossy().to_string());
            }
        }

        if let Some(p) = prompt {
            if let Some(template) = &profile.prompt_template {
                cmd.push(template.replace("{prompt}", p));
            } else {
                cmd.push(p.to_string());
            }
        }

        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn resolve_command_does_not_append_cwd_by_default() {
        let mut config = AgentConfig::default();
        config.agents.insert(
            "agent".to_string(),
            AgentProfile {
                command: "agent".to_string(),
                ..Default::default()
            },
        );
        let cwd = Path::new("/tmp/myproject");
        let cmd = config.resolve_command("agent", &[], None, Some(cwd)).unwrap();
        assert_eq!(cmd, vec!["agent"]);
    }

    #[test]
    fn resolve_command_appends_cwd_when_pass_cwd_enabled() {
        let mut config = AgentConfig::default();
        config.agents.insert(
            "agent".to_string(),
            AgentProfile {
                command: "agent".to_string(),
                pass_cwd: true,
                ..Default::default()
            },
        );
        let cwd = Path::new("/tmp/myproject");
        let cmd = config.resolve_command("agent", &[], None, Some(cwd)).unwrap();
        assert_eq!(cmd, vec!["agent", "/tmp/myproject"]);
    }
}

fn substitute_cwd(args: &[String], cwd: Option<&Path>) -> Vec<String> {
    let cwd_str = cwd.map(|p| p.to_string_lossy().to_string());
    args.iter()
        .map(|arg| {
            if arg.contains("{cwd}") {
                arg.replace("{cwd}", cwd_str.as_deref().unwrap_or("."))
            } else {
                arg.clone()
            }
        })
        .collect()
}
