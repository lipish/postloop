use std::fs;
use std::path::{Path, PathBuf};

/// Get list of deployed versions sorted by modification time (newest first)
pub fn get_deployed_versions(target_dir: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let path = Path::new(target_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut versions = Vec::new();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        // Skip the 'current' symlink
        if path.file_name() == Some(std::ffi::OsStr::new("current")) {
            continue;
        }

        // Only include directories
        if path.is_dir() {
            if let Some(name) = path.file_name() {
                if let Some(name_str) = name.to_str() {
                    versions.push((name_str.to_string(), entry.metadata()?.modified()?));
                }
            }
        }
    }

    // Sort by modification time (newest first)
    versions.sort_by(|a, b| b.1.cmp(&a.1));

    Ok(versions.into_iter().map(|(name, _)| name).collect())
}

/// Clean up old versions, keeping only the specified number
pub fn cleanup_old_versions(
    target_dir: &str,
    keep_versions: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let versions = get_deployed_versions(target_dir)?;

    if versions.len() <= keep_versions {
        log::info!(
            "No cleanup needed: {} versions, keeping {}",
            versions.len(),
            keep_versions
        );
        return Ok(());
    }

    // Remove old versions
    for version in versions.iter().skip(keep_versions) {
        let mut version_path = PathBuf::from(target_dir);
        version_path.push(version);

        log::info!("Removing old version: {:?}", version_path);
        fs::remove_dir_all(&version_path)?;
    }

    Ok(())
}

/// Rollback to previous version
pub fn rollback_to_previous(target_dir: &str) -> Result<String, Box<dyn std::error::Error>> {
    let versions = get_deployed_versions(target_dir)?;

    if versions.len() < 2 {
        return Err("No previous version available for rollback".into());
    }

    // The first version is the current one, so we want the second one
    let previous_version = &versions[1];

    // Update 'current' symlink to point to previous version
    let current_link = format!("{}/current", target_dir);
    let previous_path = format!("{}/{}", target_dir, previous_version);

    // Remove existing symlink
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

    // Create new symlink to previous version
    #[cfg(unix)]
    std::os::unix::fs::symlink(&previous_path, &current_link)?;
    
    #[cfg(windows)]
    {
        if Path::new(&previous_path).is_dir() {
            std::os::windows::fs::symlink_dir(&previous_path, &current_link)?;
        } else {
            std::os::windows::fs::symlink_file(&previous_path, &current_link)?;
        }
    }

    log::info!("Rolled back to version: {}", previous_version);

    Ok(previous_version.clone())
}

/// Rollback to a specific version
pub fn rollback_to_version(
    target_dir: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let version_path = format!("{}/{}", target_dir, version);

    if !Path::new(&version_path).exists() {
        return Err(format!("Version not found: {}", version).into());
    }

    // Update 'current' symlink
    let current_link = format!("{}/current", target_dir);

    // Remove existing symlink
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
    std::os::unix::fs::symlink(&version_path, &current_link)?;
    
    #[cfg(windows)]
    {
        if Path::new(&version_path).is_dir() {
            std::os::windows::fs::symlink_dir(&version_path, &current_link)?;
        } else {
            std::os::windows::fs::symlink_file(&version_path, &current_link)?;
        }
    }

    log::info!("Rolled back to version: {}", version);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_deployed_versions() {
        // Just test that the function doesn't panic
        let result = get_deployed_versions("/tmp/nonexistent");
        assert!(result.is_ok());
    }
}
