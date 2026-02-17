use std::process::Command;

/// Execute build command
pub fn build(command: &str, repo_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Starting build with command: {}", command);

    // Parse command into parts
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Build command is empty".into());
    }

    let program = parts[0];
    let args = &parts[1..];

    // Execute build command
    let output = Command::new(program)
        .args(args)
        .current_dir(repo_path)
        .output()?;

    // Check if build succeeded
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("Build failed: {}", stderr);
        return Err(format!("Build failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    log::info!("Build succeeded: {}", stdout);

    Ok(())
}

/// Verify that build artifacts exist
pub fn verify_artifacts(artifacts: &[String], repo_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    for artifact in artifacts {
        let mut artifact_path = std::path::PathBuf::from(repo_path);
        artifact_path.push(artifact);

        if !artifact_path.exists() {
            return Err(format!("Build artifact not found: {}", artifact).into());
        }

        log::info!("Verified artifact: {}", artifact);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_with_echo() {
        let result = build("echo test", ".");
        assert!(result.is_ok());
    }
}
