use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub watch: WatchConfig,
    pub build: BuildConfig,
    pub deploy: DeployConfig,
    pub sync: SyncConfig,
    pub rollback: RollbackConfig,
    pub log: LogConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WatchConfig {
    pub repo_path: String,
    pub branch: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BuildConfig {
    pub command: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeployConfig {
    pub command: Option<String>,
    pub target_dir: Option<String>,
    pub artifacts: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SyncConfig {
    pub enabled: bool,
    pub remote: String,
    pub branch: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RollbackConfig {
    pub enabled: bool,
    pub keep_versions: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LogConfig {
    pub file: String,
    pub level: String,
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Generate default configuration
    pub fn default() -> Self {
        Config {
            watch: WatchConfig {
                repo_path: ".".to_string(),
                branch: "main".to_string(),
            },
            build: BuildConfig {
                command: "cargo build --release".to_string(),
            },
            deploy: DeployConfig {
                command: None,
                target_dir: Some("/opt/deploy".to_string()),
                artifacts: Some(vec!["target/release/my-app".to_string()]),
            },
            sync: SyncConfig {
                enabled: true,
                remote: "origin".to_string(),
                branch: "main".to_string(),
            },
            rollback: RollbackConfig {
                enabled: true,
                keep_versions: 3,
            },
            log: LogConfig {
                file: "ploop.log".to_string(),
                level: "info".to_string(),
            },
        }
    }

    /// Save configuration to a TOML file
    pub fn save(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Check if configuration file exists
    pub fn exists(path: &str) -> bool {
        Path::new(path).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.watch.branch, "main");
        assert_eq!(config.build.command, "cargo build --release");
        assert_eq!(config.sync.enabled, true);
        assert_eq!(config.rollback.keep_versions, 3);
    }
}
