use crate::error::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Read a file's content if it exists.
///
/// Returns `Ok(Some(content))` if the file exists and is readable.
/// Returns `Ok(None)` if the file does not exist.
/// Returns `Err` for permission or other IO errors.
pub fn read_if_exists(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|e| {
        Error::Integration(format!("Failed to read {}: {e}", path.display()))
    })?;
    Ok(Some(content))
}

fn adjacent_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| format!("{}.{suffix}", name.to_string_lossy()))
        .unwrap_or_else(|| format!(".{suffix}"));
    path.with_file_name(file_name)
}

/// Create a backup of a file (one-time .bak).
///
/// Only creates a backup if the .bak file does not already exist,
/// so repeated operations don't overwrite the original backup.
pub fn backup_if_needed(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let backup_path = adjacent_path(path, "bak");
    if backup_path.exists() {
        // Already have a backup; don't overwrite it
        return Ok(());
    }

    fs::copy(path, &backup_path).map_err(|e| {
        Error::Integration(format!(
            "Failed to create backup {} -> {}: {e}",
            path.display(),
            backup_path.display()
        ))
    })?;

    Ok(())
}

/// Write content to a file atomically (write temp, then rename).
///
/// Creates parent directories if needed.
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            Error::Integration(format!(
                "Failed to create directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let temp_path = adjacent_path(path, "tmp");
    fs::write(&temp_path, content).map_err(|e| {
        Error::Integration(format!("Failed to write temp file {}: {e}", temp_path.display()))
    })?;

    fs::rename(&temp_path, path).map_err(|e| {
        // Clean up temp file on rename failure
        let _ = fs::remove_file(&temp_path);
        Error::Integration(format!(
            "Failed to rename {} -> {}: {e}",
            temp_path.display(),
            path.display()
        ))
    })?;

    Ok(())
}

/// Check if a file contains only whitespace (or is empty).
pub fn is_effectively_empty(content: &str) -> bool {
    content.trim().is_empty()
}

/// Remove a file if it exists.
pub fn remove_file_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| {
            Error::Integration(format!("Failed to remove {}: {e}", path.display()))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_read_if_exists_present() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.md");
        fs::write(&path, "hello").unwrap();

        let result = read_if_exists(&path).unwrap();
        assert_eq!(result, Some("hello".to_string()));
    }

    #[test]
    fn test_read_if_exists_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.md");

        let result = read_if_exists(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_backup_if_needed_creates_backup() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.md");
        fs::write(&path, "original").unwrap();

        backup_if_needed(&path).unwrap();

        let backup_path = path.with_extension("md.bak");
        assert!(backup_path.exists());
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "original");
    }

    #[test]
    fn test_backup_if_needed_does_not_overwrite() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.md");
        fs::write(&path, "original").unwrap();

        backup_if_needed(&path).unwrap();

        // Modify original
        fs::write(&path, "modified").unwrap();

        // Second backup should not overwrite
        backup_if_needed(&path).unwrap();

        let backup_path = path.with_extension("md.bak");
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "original");
    }

    #[test]
    fn test_backup_if_needed_preserves_original_extension() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "original").unwrap();

        backup_if_needed(&path).unwrap();

        let backup_path = path.with_file_name("notes.txt.bak");
        assert!(backup_path.exists());
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "original");
    }

    #[test]
    fn test_backup_if_needed_no_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.md");
        backup_if_needed(&path).unwrap(); // should not error
    }

    #[test]
    fn test_write_atomic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("output.md");

        write_atomic(&path, "content here").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "content here");
        // Temp file should be cleaned up
        assert!(!path.with_extension("md.tmp").exists());
    }

    #[test]
    fn test_write_atomic_creates_parents() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub").join("dir").join("output.md");

        write_atomic(&path, "nested").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "nested");
    }

    #[test]
    fn test_write_atomic_uses_matching_temp_name() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("notes.txt");

        write_atomic(&path, "content here").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "content here");
        assert!(!path.with_file_name("notes.txt.tmp").exists());
    }

    #[test]
    fn test_is_effectively_empty() {
        assert!(is_effectively_empty(""));
        assert!(is_effectively_empty("  \n  \n  "));
        assert!(!is_effectively_empty("content"));
        assert!(!is_effectively_empty("  content  "));
    }

    #[test]
    fn test_remove_file_if_exists() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.md");
        fs::write(&path, "data").unwrap();
        assert!(path.exists());

        remove_file_if_exists(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_remove_file_if_exists_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.md");
        remove_file_if_exists(&path).unwrap(); // should not error
    }
}
