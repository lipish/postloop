use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Deploy using a custom command (process deployment)
pub fn deploy_with_command(command: &str, repo_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Starting deployment with command: {}", command);

    // Parse command into parts
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Deploy command is empty".into());
    }

    let program = parts[0];
    let args = &parts[1..];

    // Execute deploy command
    let output = Command::new(program)
        .args(args)
        .current_dir(repo_path)
        .output()?;

    // Check if deployment succeeded
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("Deployment failed: {}", stderr);
        return Err(format!("Deployment failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    log::info!("Deployment succeeded: {}", stdout);

    Ok(())
}

/// Deploy by copying artifacts to target directory (file deployment)
pub fn deploy_with_files(
    artifacts: &[String],
    target_dir: &str,
    repo_path: &str,
    commit_hash: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Starting file deployment to: {}", target_dir);

    // Create versioned target directory
    let versioned_dir = format!("{}/{}", target_dir, commit_hash);
    fs::create_dir_all(&versioned_dir)?;

    // Copy artifacts to versioned directory
    for artifact in artifacts {
        let mut src_path = PathBuf::from(repo_path);
        src_path.push(artifact);

        if !src_path.exists() {
            return Err(format!("Artifact not found: {}", artifact).into());
        }

        let file_name = src_path.file_name().ok_or("Invalid artifact path")?;
        let mut dest_path = PathBuf::from(&versioned_dir);
        dest_path.push(file_name);

        fs::copy(&src_path, &dest_path)?;
        log::info!("Copied artifact: {} -> {:?}", artifact, dest_path);
    }

    // Create or update 'current' symlink to point to the latest version
    let current_link = format!("{}/current", target_dir);
    
    // Remove existing symlink if it exists
    if Path::new(&current_link).exists() {
        #[cfg(unix)]
        fs::remove_file(&current_link)?;
        #[cfg(windows)]
        {
            if Path::new(&current_link).is_dir() {
                fs::remove_dir(&current_link)?;
            } else {
                fs::remove_file(&current_link)?;
            }
        }
    }

    // Create new symlink
    #[cfg(unix)]
    std::os::unix::fs::symlink(&versioned_dir, &current_link)?;
    
    #[cfg(windows)]
    {
        if Path::new(&versioned_dir).is_dir() {
            std::os::windows::fs::symlink_dir(&versioned_dir, &current_link)?;
        } else {
            std::os::windows::fs::symlink_file(&versioned_dir, &current_link)?;
        }
    }

    log::info!("Updated 'current' symlink to: {}", versioned_dir);

    Ok(())
}

/// Deploy artifacts (choose between command or file deployment)
pub fn deploy(
    command: Option<&str>,
    artifacts: Option<&[String]>,
    target_dir: Option<&str>,
    repo_path: &str,
    commit_hash: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Try command deployment first
    if let Some(cmd) = command {
        return deploy_with_command(cmd, repo_path);
    }

    // Fall back to file deployment
    if let (Some(arts), Some(target)) = (artifacts, target_dir) {
        return deploy_with_files(arts, target, repo_path, commit_hash);
    }

    Err("No deployment method configured (neither command nor target_dir/artifacts)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deploy_with_echo() {
        let result = deploy_with_command("echo deployed", ".");
        assert!(result.is_ok());
    }
}
