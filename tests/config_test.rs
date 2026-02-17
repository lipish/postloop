#[cfg(test)]
mod config_tests {
    use postloop::config::Config;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.watch.repo_path, ".");
        assert_eq!(config.watch.branch, "main");
        assert_eq!(config.build.command, "cargo build --release");
        assert!(config.sync.enabled);
        assert_eq!(config.rollback.keep_versions, 3);
    }

    #[test]
    fn test_config_exists() {
        assert!(!Config::exists("/nonexistent/path/deploy.toml"));
    }
}
