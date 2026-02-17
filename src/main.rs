mod builder;
mod config;
mod deployer;
mod hook;
mod logger;
mod rollback;
mod syncer;

use clap::{Parser, Subcommand};
use std::path::Path;

#[derive(Parser)]
#[command(name = "ploop")]
#[command(about = "Post-commit Loop - Local Git auto-deployment tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize ploop in current Git repository
    Init,
    /// Run deployment pipeline manually
    Run,
    /// Rollback to previous version
    Rollback {
        /// Specific version to rollback to (optional)
        #[arg(short, long)]
        version: Option<String>,
    },
    /// Show deployment status and history
    Status,
    /// Show deployment logs
    Log {
        /// Number of lines to show
        #[arg(short, long, default_value = "50")]
        lines: usize,
    },
}

fn main() {
    // Initialize simple logger for console output
    logger::init_simple_logger();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init => cmd_init(),
        Commands::Run => cmd_run(),
        Commands::Rollback { version } => cmd_rollback(version),
        Commands::Status => cmd_status(),
        Commands::Log { lines } => cmd_log(lines),
    };

    if let Err(e) = result {
        log::error!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_init() -> Result<(), Box<dyn std::error::Error>> {
    let repo_path = ".";

    // Check if we're in a Git repository
    if !hook::is_git_repo(repo_path) {
        return Err("Not a Git repository. Please run 'git init' first.".into());
    }

    // Create default config if it doesn't exist
    let config_path = "deploy.toml";
    if !config::Config::exists(config_path) {
        let default_config = config::Config::default();
        default_config.save(config_path)?;
        println!("✓ Created default configuration file: {}", config_path);
    } else {
        println!("✓ Configuration file already exists: {}", config_path);
    }

    // Install post-commit hook
    if hook::is_hook_installed(repo_path) {
        println!("✓ Post-commit hook already installed");
    } else {
        hook::install_hook(repo_path)?;
        println!("✓ Post-commit hook installed");
    }

    println!("\nInitialization complete!");
    println!("Edit {} to configure your deployment", config_path);

    Ok(())
}

fn cmd_run() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = "deploy.toml";
    
    // Load configuration
    let config = config::Config::load(config_path)?;

    // Initialize file logger
    logger::PloopLogger::init(&config.log.file, &config.log.level)?;

    let repo_path = &config.watch.repo_path;

    // Get current commit hash
    let short_hash = hook::get_short_commit_hash(repo_path)?;

    log::info!("Starting deployment for commit: {}", short_hash);
    println!("🚀 Starting deployment for commit: {}", short_hash);

    // Step 1: Build
    println!("📦 Building...");
    if let Err(e) = builder::build(&config.build.command, repo_path) {
        log::error!("Build failed: {}", e);
        println!("❌ Build failed: {}", e);
        return Err(e);
    }
    println!("✓ Build succeeded");

    // Verify artifacts if configured
    if let Some(ref artifacts) = config.deploy.artifacts {
        if let Err(e) = builder::verify_artifacts(artifacts, repo_path) {
            log::error!("Artifact verification failed: {}", e);
            println!("❌ Artifact verification failed: {}", e);
            return Err(e);
        }
    }

    // Step 2: Deploy
    println!("🚢 Deploying...");
    let deploy_result = deployer::deploy(
        config.deploy.command.as_deref(),
        config.deploy.artifacts.as_deref(),
        config.deploy.target_dir.as_deref(),
        repo_path,
        &short_hash,
    );

    if let Err(e) = deploy_result {
        log::error!("Deployment failed: {}", e);
        println!("❌ Deployment failed: {}", e);

        // Attempt rollback if enabled and using file deployment
        if config.rollback.enabled && config.deploy.target_dir.is_some() {
            println!("🔄 Attempting rollback...");
            if let Some(ref target_dir) = config.deploy.target_dir {
                if let Ok(prev_version) = rollback::rollback_to_previous(target_dir) {
                    log::info!("Rolled back to version: {}", prev_version);
                    println!("✓ Rolled back to version: {}", prev_version);
                } else {
                    log::warn!("Rollback failed: no previous version available");
                    println!("⚠ Rollback failed: no previous version available");
                }
            }
        }

        return Err(e);
    }
    println!("✓ Deployment succeeded");

    // Clean up old versions if enabled
    if config.rollback.enabled {
        if let Some(ref target_dir) = config.deploy.target_dir {
            if let Err(e) = rollback::cleanup_old_versions(target_dir, config.rollback.keep_versions)
            {
                log::warn!("Failed to cleanup old versions: {}", e);
            }
        }
    }

    // Step 3: Sync to GitHub
    if config.sync.enabled {
        println!("☁️  Syncing to GitHub...");
        match syncer::sync_to_github(&config.sync.remote, &config.sync.branch, repo_path) {
            Ok(_) => {
                log::info!("GitHub sync succeeded");
                println!("✓ Synced to GitHub");
            }
            Err(e) => {
                log::warn!("GitHub sync failed: {}", e);
                println!("⚠ GitHub sync failed: {}", e);
                println!("  (Deployment was successful, but push failed)");
            }
        }
    }

    println!("\n✅ Deployment complete for commit: {}", short_hash);
    log::info!("Deployment complete for commit: {}", short_hash);

    Ok(())
}

fn cmd_rollback(version: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = "deploy.toml";
    let config = config::Config::load(config_path)?;

    if !config.rollback.enabled {
        return Err("Rollback is not enabled in configuration".into());
    }

    let target_dir = config
        .deploy
        .target_dir
        .ok_or("No target_dir configured for rollback")?;

    if let Some(ver) = version {
        println!("🔄 Rolling back to version: {}", ver);
        rollback::rollback_to_version(&target_dir, &ver)?;
        println!("✓ Rolled back to version: {}", ver);
    } else {
        println!("🔄 Rolling back to previous version...");
        let prev_version = rollback::rollback_to_previous(&target_dir)?;
        println!("✓ Rolled back to version: {}", prev_version);
    }

    Ok(())
}

fn cmd_status() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = "deploy.toml";
    let config = config::Config::load(config_path)?;

    let repo_path = &config.watch.repo_path;

    println!("📊 Deployment Status\n");

    // Current commit
    let short_hash = hook::get_short_commit_hash(repo_path)?;
    println!("Current commit: {}", short_hash);

    // Hook status
    let hook_installed = hook::is_hook_installed(repo_path);
    println!(
        "Post-commit hook: {}",
        if hook_installed {
            "✓ Installed"
        } else {
            "✗ Not installed"
        }
    );

    // Deployment versions
    if let Some(ref target_dir) = config.deploy.target_dir {
        if Path::new(target_dir).exists() {
            println!("\nDeployed versions:");
            let versions = rollback::get_deployed_versions(target_dir)?;
            for (i, version) in versions.iter().enumerate() {
                let marker = if i == 0 { "→" } else { " " };
                println!("  {} {}", marker, version);
            }
        } else {
            println!("\nNo deployments found");
        }
    }

    // Sync status
    if config.sync.enabled {
        println!("\nGitHub sync: enabled");
        match syncer::has_unpushed_commits(&config.sync.remote, &config.sync.branch, repo_path) {
            Ok(true) => println!("  ⚠ Has unpushed commits"),
            Ok(false) => println!("  ✓ Up to date"),
            Err(e) => println!("  ⚠ Could not check status: {}", e),
        }
    }

    Ok(())
}

fn cmd_log(lines: usize) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = "deploy.toml";
    let config = config::Config::load(config_path)?;

    let log_file = &config.log.file;

    if !Path::new(log_file).exists() {
        println!("No log file found: {}", log_file);
        return Ok(());
    }

    let content = std::fs::read_to_string(log_file)?;
    let all_lines: Vec<&str> = content.lines().collect();

    let start = if all_lines.len() > lines {
        all_lines.len() - lines
    } else {
        0
    };

    println!("📋 Deployment Log (last {} lines):\n", lines);
    for line in &all_lines[start..] {
        println!("{}", line);
    }

    Ok(())
}
