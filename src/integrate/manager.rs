use crate::error::{Error, Result};
use crate::integrate::fs as safe_fs;
use crate::integrate::markers;
use crate::integrate::render;
use crate::integrate::targets::{InstallPolicy, IntegrationTarget};
use std::path::{Path, PathBuf};

/// Result of an integration operation.
#[derive(Debug)]
pub struct IntegrationResult {
    /// The target that was operated on.
    pub target: String,
    /// The file path that was modified (or would be modified in dry-run).
    pub path: PathBuf,
    /// What action was taken.
    pub action: IntegrationAction,
}

/// The action taken by an integration operation.
#[derive(Debug, PartialEq, Eq)]
pub enum IntegrationAction {
    /// Block was inserted into an existing file.
    Inserted,
    /// Block was replaced (refreshed) in the file.
    Replaced,
    /// Block was removed from the file.
    Removed,
    /// File was created with the managed block.
    Created,
    /// File was removed because it contained only the managed block.
    FileRemoved,
    /// No changes were needed (already up to date / already absent).
    NoChange,
    /// Dry run: would have performed the given action.
    DryRun(Box<IntegrationAction>),
}

impl std::fmt::Display for IntegrationAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntegrationAction::Inserted => write!(f, "Inserted managed block"),
            IntegrationAction::Replaced => write!(f, "Replaced managed block"),
            IntegrationAction::Removed => write!(f, "Removed managed block"),
            IntegrationAction::Created => write!(f, "Created file with managed block"),
            IntegrationAction::FileRemoved => write!(f, "Removed file (was only managed content)"),
            IntegrationAction::NoChange => write!(f, "No changes needed"),
            IntegrationAction::DryRun(inner) => write!(f, "Would: {inner}"),
        }
    }
}

/// Status information for a target integration.
#[derive(Debug)]
pub struct IntegrationStatus {
    pub target: String,
    pub path: PathBuf,
    pub installed: bool,
    pub file_exists: bool,
    pub marker_health: MarkerHealth,
    pub topic_count: Option<usize>,
}

/// Health status of markers in a target file.
#[derive(Debug, PartialEq, Eq)]
pub enum MarkerHealth {
    /// Markers are correctly formed with matched begin/end.
    Healthy,
    /// No managed block present.
    NotInstalled,
    /// Markers are malformed (with description).
    Malformed(String),
    /// Target file doesn't exist.
    FileAbsent,
}

impl std::fmt::Display for IntegrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Target: {}", self.target)?;
        writeln!(f, "Path: {}", self.path.display())?;
        writeln!(f, "File exists: {}", self.file_exists)?;
        writeln!(f, "Installed: {}", self.installed)?;
        match &self.marker_health {
            MarkerHealth::Healthy => writeln!(f, "Marker health: OK")?,
            MarkerHealth::NotInstalled => writeln!(f, "Marker health: not installed")?,
            MarkerHealth::Malformed(detail) => writeln!(f, "Marker health: MALFORMED - {detail}")?,
            MarkerHealth::FileAbsent => writeln!(f, "Marker health: file absent")?,
        }
        if let Some(count) = self.topic_count {
            writeln!(f, "Topics in block: {count}")?;
        }
        Ok(())
    }
}

/// Install a managed block at a specific path.
///
/// Creates the file if absent, inserts or replaces the block if present.
/// Returns a display string describing the action taken.
pub fn install_block_at(
    path: &Path,
    target_id: &str,
    block: &str,
) -> Result<IntegrationAction> {
    let existing = safe_fs::read_if_exists(path)?;

    match existing {
        Some(content) => {
            let location = markers::find_block(&content, target_id, &path.display().to_string())?;
            match location {
                Some(loc) => {
                    let new_content = markers::replace_block(&content, &loc, block);
                    if new_content == content {
                        return Ok(IntegrationAction::NoChange);
                    }
                    safe_fs::write_atomic(path, &new_content)?;
                    Ok(IntegrationAction::Replaced)
                }
                None => {
                    let new_content = markers::insert_block(&content, block);
                    safe_fs::write_atomic(path, &new_content)?;
                    Ok(IntegrationAction::Inserted)
                }
            }
        }
        None => {
            safe_fs::write_atomic(path, block)?;
            Ok(IntegrationAction::Created)
        }
    }
}

/// Install the managed block into a target file.
///
/// If the block already exists, it is replaced (idempotent).
/// If the file doesn't exist and install policy allows, the file is created.
pub fn install(
    target: &IntegrationTarget,
    path_override: Option<&Path>,
    dry_run: bool,
) -> Result<IntegrationResult> {
    let path = target.resolve_path(path_override);
    crate::integrate::targets::validate_target_path(&path)?;

    let block = render::render_managed_block(target);
    let existing = safe_fs::read_if_exists(&path)?;

    match existing {
        Some(content) => {
            // File exists: insert or replace
            let location = markers::find_block(&content, &target.identifier, &path.display().to_string())?;

            let new_content = match location {
                Some(loc) => {
                    // Block exists: replace
                    let new_content = markers::replace_block(&content, &loc, &block);
                    if new_content == content {
                        return Ok(IntegrationResult {
                            target: target.identifier.clone(),
                            path,
                            action: if dry_run {
                                IntegrationAction::DryRun(Box::new(IntegrationAction::NoChange))
                            } else {
                                IntegrationAction::NoChange
                            },
                        });
                    }
                    (new_content, IntegrationAction::Replaced)
                }
                None => {
                    // No block: insert at end
                    let new_content = markers::insert_block(&content, &block);
                    (new_content, IntegrationAction::Inserted)
                }
            };

            if dry_run {
                return Ok(IntegrationResult {
                    target: target.identifier.clone(),
                    path,
                    action: IntegrationAction::DryRun(Box::new(new_content.1)),
                });
            }

            safe_fs::backup_if_needed(&path)?;
            safe_fs::write_atomic(&path, &new_content.0)?;

            Ok(IntegrationResult {
                target: target.identifier.clone(),
                path,
                action: new_content.1,
            })
        }
        None => {
            // File doesn't exist
            match target.install_policy {
                InstallPolicy::CreateIfAbsent => {
                    if dry_run {
                        return Ok(IntegrationResult {
                            target: target.identifier.clone(),
                            path,
                            action: IntegrationAction::DryRun(Box::new(IntegrationAction::Created)),
                        });
                    }
                    safe_fs::write_atomic(&path, &block)?;
                    Ok(IntegrationResult {
                        target: target.identifier.clone(),
                        path,
                        action: IntegrationAction::Created,
                    })
                }
                InstallPolicy::RequireExisting => Err(Error::Integration(format!(
                    "Target file does not exist and install policy requires existing file: {}",
                    path.display()
                ))),
            }
        }
    }
}

/// Refresh the managed block in a target file.
///
/// Only operates on files that already have a managed block installed.
/// Returns NoChange if no block is found.
pub fn refresh(
    target: &IntegrationTarget,
    path_override: Option<&Path>,
    dry_run: bool,
) -> Result<IntegrationResult> {
    let path = target.resolve_path(path_override);
    let existing = safe_fs::read_if_exists(&path)?;

    match existing {
        Some(content) => {
            let location =
                markers::find_block(&content, &target.identifier, &path.display().to_string())?;

            match location {
                Some(loc) => {
                    let block = render::render_managed_block(target);
                    let new_content = markers::replace_block(&content, &loc, &block);

                    if new_content == content {
                        return Ok(IntegrationResult {
                            target: target.identifier.clone(),
                            path,
                            action: if dry_run {
                                IntegrationAction::DryRun(Box::new(IntegrationAction::NoChange))
                            } else {
                                IntegrationAction::NoChange
                            },
                        });
                    }

                    if dry_run {
                        return Ok(IntegrationResult {
                            target: target.identifier.clone(),
                            path,
                            action: IntegrationAction::DryRun(Box::new(IntegrationAction::Replaced)),
                        });
                    }

                    safe_fs::backup_if_needed(&path)?;
                    safe_fs::write_atomic(&path, &new_content)?;

                    Ok(IntegrationResult {
                        target: target.identifier.clone(),
                        path,
                        action: IntegrationAction::Replaced,
                    })
                }
                None => Ok(IntegrationResult {
                    target: target.identifier.clone(),
                    path,
                    action: if dry_run {
                        IntegrationAction::DryRun(Box::new(IntegrationAction::NoChange))
                    } else {
                        IntegrationAction::NoChange
                    },
                }),
            }
        }
        None => Ok(IntegrationResult {
            target: target.identifier.clone(),
            path,
            action: if dry_run {
                IntegrationAction::DryRun(Box::new(IntegrationAction::NoChange))
            } else {
                IntegrationAction::NoChange
            },
        }),
    }
}

/// Remove the managed block from a target file.
///
/// If the file becomes empty after removal, the file itself is deleted.
/// Idempotent: returns NoChange if no block is found.
pub fn remove(
    target: &IntegrationTarget,
    path_override: Option<&Path>,
    dry_run: bool,
) -> Result<IntegrationResult> {
    let path = target.resolve_path(path_override);
    let existing = safe_fs::read_if_exists(&path)?;

    match existing {
        Some(content) => {
            let location =
                markers::find_block(&content, &target.identifier, &path.display().to_string())?;

            match location {
                Some(loc) => {
                    let new_content = markers::remove_block(&content, &loc);

                    if safe_fs::is_effectively_empty(&new_content) {
                        if dry_run {
                            return Ok(IntegrationResult {
                                target: target.identifier.clone(),
                                path,
                                action: IntegrationAction::DryRun(Box::new(
                                    IntegrationAction::FileRemoved,
                                )),
                            });
                        }

                        safe_fs::remove_file_if_exists(&path)?;

                        Ok(IntegrationResult {
                            target: target.identifier.clone(),
                            path,
                            action: IntegrationAction::FileRemoved,
                        })
                    } else {
                        if dry_run {
                            return Ok(IntegrationResult {
                                target: target.identifier.clone(),
                                path,
                                action: IntegrationAction::DryRun(Box::new(
                                    IntegrationAction::Removed,
                                )),
                            });
                        }

                        safe_fs::backup_if_needed(&path)?;
                        safe_fs::write_atomic(&path, &new_content)?;

                        Ok(IntegrationResult {
                            target: target.identifier.clone(),
                            path,
                            action: IntegrationAction::Removed,
                        })
                    }
                }
                None => Ok(IntegrationResult {
                    target: target.identifier.clone(),
                    path,
                    action: if dry_run {
                        IntegrationAction::DryRun(Box::new(IntegrationAction::NoChange))
                    } else {
                        IntegrationAction::NoChange
                    },
                }),
            }
        }
        None => Ok(IntegrationResult {
            target: target.identifier.clone(),
            path,
            action: if dry_run {
                IntegrationAction::DryRun(Box::new(IntegrationAction::NoChange))
            } else {
                IntegrationAction::NoChange
            },
        }),
    }
}

/// Get the status of a target integration.
pub fn status(
    target: &IntegrationTarget,
    path_override: Option<&Path>,
) -> IntegrationStatus {
    let path = target.resolve_path(path_override);
    let file_exists = path.exists();

    if !file_exists {
        return IntegrationStatus {
            target: target.identifier.clone(),
            path,
            installed: false,
            file_exists: false,
            marker_health: MarkerHealth::FileAbsent,
            topic_count: None,
        };
    }

    let content = match safe_fs::read_if_exists(&path) {
        Ok(Some(c)) => c,
        _ => {
            return IntegrationStatus {
                target: target.identifier.clone(),
                path,
                installed: false,
                file_exists: true,
                marker_health: MarkerHealth::Malformed("Unable to read file".to_string()),
                topic_count: None,
            };
        }
    };

    match markers::find_block(&content, &target.identifier, &path.display().to_string()) {
        Ok(Some(loc)) => {
            let block_content = &content[loc.start..loc.end];
            // Count topics by looking for "Current topics:" line
            let topic_count = extract_topic_count(block_content);

            IntegrationStatus {
                target: target.identifier.clone(),
                path,
                installed: true,
                file_exists: true,
                marker_health: MarkerHealth::Healthy,
                topic_count: Some(topic_count),
            }
        }
        Ok(None) => IntegrationStatus {
            target: target.identifier.clone(),
            path,
            installed: false,
            file_exists: true,
            marker_health: MarkerHealth::NotInstalled,
            topic_count: None,
        },
        Err(e) => IntegrationStatus {
            target: target.identifier.clone(),
            path,
            installed: false,
            file_exists: true,
            marker_health: MarkerHealth::Malformed(e.to_string()),
            topic_count: None,
        },
    }
}

/// Extract the count of topics from a managed block's content.
fn extract_topic_count(block_content: &str) -> usize {
    for line in block_content.lines() {
        if let Some(rest) = line.strip_prefix("Current topics: ") {
            return rest.split(", ").count();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrate::targets::IntegrationTarget;
    use crate::kb::types::TopicSummary;
    use std::fs;
    use tempfile::TempDir;

    fn make_topics(slugs: &[&str]) -> Vec<TopicSummary> {
        slugs
            .iter()
            .map(|s| TopicSummary {
                slug: s.to_string(),
                title: s.to_string(),
                aliases: Vec::new(),
                tags: Vec::new(),
            })
            .collect()
    }

    fn target_at(dir: &TempDir, filename: &str) -> (IntegrationTarget, PathBuf) {
        let path = dir.path().join(filename);
        let mut target = IntegrationTarget::claude_code();
        target.default_path = path.clone();
        (target, path)
    }

    #[test]
    fn test_install_creates_new_file() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");
        let topics = make_topics(&["gh-cli", "aws-sso"]);

        let result = install(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::Created);
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("oversight:begin target=claude-code"));
        assert!(content.contains("oversight topics"));
        assert!(content.contains("oversight:end"));
    }

    #[test]
    fn test_install_into_existing_file() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");

        fs::write(&path, "# My Config\n\nExisting content.\n").unwrap();

        let result = install(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::Inserted);

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("# My Config"));
        assert!(content.contains("Existing content."));
        assert!(content.contains("oversight:begin target=claude-code"));
        assert!(content.contains("oversight topics"));
    }

    #[test]
    fn test_install_idempotent() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");
        let topics = make_topics(&["gh-cli"]);

        install(&target, None, false).unwrap();
        let content_after_first = fs::read_to_string(&path).unwrap();

        let result = install(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::NoChange);

        let content_after_second = fs::read_to_string(&path).unwrap();
        assert_eq!(content_after_first, content_after_second);
    }

    #[test]
    fn test_install_is_idempotent_with_static_block() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");

        install(&target, None, false).unwrap();
        let result = install(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::NoChange);

        assert_eq!(fs::read_to_string(&path).unwrap().matches("oversight:begin").count(), 1);
    }

    #[test]
    fn test_install_dry_run() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");
        let topics = make_topics(&["gh-cli"]);

        let result = install(&target, None, true).unwrap();
        assert!(matches!(result.action, IntegrationAction::DryRun(_)));
        assert!(!path.exists(), "Dry run should not create file");
    }

    #[test]
    fn test_refresh_is_no_change_when_block_unchanged() {
        let dir = TempDir::new().unwrap();
        let (target, _path) = target_at(&dir, "CLAUDE.md");

        install(&target, None, false).unwrap();
        let result = refresh(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::NoChange);
    }

    #[test]
    fn test_refresh_no_block_returns_no_change() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");
        fs::write(&path, "# My Config\n").unwrap();

        let topics = make_topics(&["gh-cli"]);
        let result = refresh(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::NoChange);

        // Original content should be unchanged
        assert_eq!(fs::read_to_string(&path).unwrap(), "# My Config\n");
    }

    #[test]
    fn test_refresh_file_absent_returns_no_change() {
        let dir = TempDir::new().unwrap();
        let (target, _path) = target_at(&dir, "CLAUDE.md");

        let topics = make_topics(&["gh-cli"]);
        let result = refresh(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::NoChange);
    }

    #[test]
    fn test_remove_deletes_block() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");

        fs::write(&path, "# My Config\n\nExisting content.\n").unwrap();
        let topics = make_topics(&["gh-cli"]);
        install(&target, None, false).unwrap();

        let result = remove(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::Removed);

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("oversight:begin"));
        assert!(!content.contains("oversight:end"));
        assert!(content.contains("My Config"));
        assert!(content.contains("Existing content."));
    }

    #[test]
    fn test_remove_deletes_file_if_only_managed() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");
        let topics = make_topics(&["gh-cli"]);

        install(&target, None, false).unwrap();
        assert!(path.exists());

        let result = remove(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::FileRemoved);
        assert!(!path.exists());
    }

    #[test]
    fn test_remove_idempotent() {
        let dir = TempDir::new().unwrap();
        let (target, _path) = target_at(&dir, "CLAUDE.md");

        let result = remove(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::NoChange);
    }

    #[test]
    fn test_remove_dry_run() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");
        let topics = make_topics(&["gh-cli"]);

        install(&target, None, false).unwrap();
        assert!(path.exists());

        let result = remove(&target, None, true).unwrap();
        assert!(matches!(result.action, IntegrationAction::DryRun(_)));
        assert!(path.exists(), "Dry run should not remove file");
    }

    #[test]
    fn test_status_not_installed() {
        let dir = TempDir::new().unwrap();
        let (target, _path) = target_at(&dir, "CLAUDE.md");

        let st = status(&target, None);
        assert!(!st.installed);
        assert!(!st.file_exists);
        assert_eq!(st.marker_health, MarkerHealth::FileAbsent);
    }

    #[test]
    fn test_status_installed() {
        let dir = TempDir::new().unwrap();
        let (target, _path) = target_at(&dir, "CLAUDE.md");
        let topics = make_topics(&["gh-cli", "aws-sso"]);

        install(&target, None, false).unwrap();

        let st = status(&target, None);
        assert!(st.installed);
        assert!(st.file_exists);
        assert_eq!(st.marker_health, MarkerHealth::Healthy);
        assert!(st.installed);
    }

    #[test]
    fn test_status_file_exists_no_block() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");
        fs::write(&path, "# Just a config\n").unwrap();

        let st = status(&target, None);
        assert!(!st.installed);
        assert!(st.file_exists);
        assert_eq!(st.marker_health, MarkerHealth::NotInstalled);
    }

    #[test]
    fn test_install_empty_kb() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");

        let result = install(&target, None, false).unwrap();
        assert_eq!(result.action, IntegrationAction::Created);

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("oversight topics"));
        assert!(content.contains("oversight:begin"));
    }

    #[test]
    fn test_install_preserves_surrounding_content() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");

        let original = "# Header\n\nSome important instructions.\n\n## Another Section\n\nMore content.\n";
        fs::write(&path, original).unwrap();

        let topics = make_topics(&["gh-cli"]);
        install(&target, None, false).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Header"));
        assert!(content.contains("Some important instructions."));
        assert!(content.contains("## Another Section"));
        assert!(content.contains("More content."));
        assert!(content.contains("oversight:begin"));
    }

    #[test]
    fn test_install_with_path_override() {
        let dir = TempDir::new().unwrap();
        let target = IntegrationTarget::claude_code();
        let custom_path = dir.path().join("custom").join("CLAUDE.md");
        let topics = make_topics(&["gh-cli"]);

        let result = install(&target, Some(&custom_path), false).unwrap();
        assert_eq!(result.path, custom_path);
        assert!(custom_path.exists());
    }

    #[test]
    fn test_backup_created_on_first_modify() {
        let dir = TempDir::new().unwrap();
        let (target, path) = target_at(&dir, "CLAUDE.md");
        let backup_path = path.with_extension("md.bak");

        // Create original file
        fs::write(&path, "# Original content\n").unwrap();
        assert!(!backup_path.exists());

        // Install should create backup
        let topics = make_topics(&["gh-cli"]);
        install(&target, None, false).unwrap();
        assert!(backup_path.exists());
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "# Original content\n");
    }

    #[test]
    fn test_require_existing_policy_errors() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agents.md");
        let mut target = IntegrationTarget::generic_agents_md(Some(path));
        target.install_policy = InstallPolicy::RequireExisting;

        let topics = make_topics(&["gh-cli"]);
        let err = install(&target, None, false).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn test_extract_topic_count() {
        assert_eq!(
            extract_topic_count("Current topics: a, b, c\n"),
            3
        );
        assert_eq!(extract_topic_count("No topics here\n"), 0);
        assert_eq!(
            extract_topic_count("Current topics: single\n"),
            1
        );
    }
}
