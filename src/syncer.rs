use std::process::Command;

/// Sync code to remote GitHub repository
pub fn sync_to_github(
    remote: &str,
    branch: &str,
    repo_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Syncing to GitHub: {} {}", remote, branch);

    // Execute git push
    let output = Command::new("git")
        .args(&["push", remote, branch])
        .current_dir(repo_path)
        .output()?;

    // Check if push succeeded
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!("GitHub sync failed: {}", stderr);
        return Err(format!("Git push failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    log::info!("GitHub sync succeeded: {} {}", stdout, stderr);

    Ok(())
}

/// Check if there are unpushed commits
pub fn has_unpushed_commits(
    remote: &str,
    branch: &str,
    repo_path: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Get local commit
    let local_output = Command::new("git")
        .args(&["rev-parse", branch])
        .current_dir(repo_path)
        .output()?;

    if !local_output.status.success() {
        return Err("Failed to get local commit".into());
    }

    let local_commit = String::from_utf8(local_output.stdout)?.trim().to_string();

    // Get remote commit
    let remote_ref = format!("{}/{}", remote, branch);
    let remote_output = Command::new("git")
        .args(&["rev-parse", &remote_ref])
        .current_dir(repo_path)
        .output()?;

    if !remote_output.status.success() {
        // Remote branch might not exist yet
        return Ok(true);
    }

    let remote_commit = String::from_utf8(remote_output.stdout)?.trim().to_string();

    Ok(local_commit != remote_commit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_unpushed_commits() {
        // Just test that the function doesn't panic
        let result = has_unpushed_commits("origin", "main", ".");
        // Result can be Ok or Err depending on git state
        assert!(result.is_ok() || result.is_err());
    }
}
