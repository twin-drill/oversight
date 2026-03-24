use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// Policy for how to handle the target file during install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallPolicy {
    /// Create the file if it does not exist.
    CreateIfAbsent,
    /// Require the file to already exist; error otherwise.
    RequireExisting,
}

/// Rendering hints that control how the managed block looks in a target.
#[derive(Debug, Clone)]
pub struct RenderingHints {
    /// The section title inside the managed block.
    pub section_title: String,
    /// How instructions are phrased (generic vs target-specific).
    pub instruction_style: InstructionStyle,
}

/// Controls how instructions are worded in the managed block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionStyle {
    /// Instructions written for Claude Code agent context.
    ClaudeCode,
    /// Generic instructions for any agent framework.
    Generic,
}

/// An integration target definition.
///
/// Each target describes where to inject managed content and how to format it.
#[derive(Debug, Clone)]
pub struct IntegrationTarget {
    /// Unique identifier for this target (e.g., "claude-code").
    pub identifier: String,
    /// Default path to the config file.
    pub default_path: PathBuf,
    /// How to render the managed section.
    pub rendering_hints: RenderingHints,
    /// Whether to create the file if absent.
    pub install_policy: InstallPolicy,
}

impl IntegrationTarget {
    /// Build the claude-code target definition.
    pub fn claude_code() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        IntegrationTarget {
            identifier: "claude-code".to_string(),
            default_path: home.join(".claude").join("CLAUDE.md"),
            rendering_hints: RenderingHints {
                section_title: "Oversight Knowledge Base".to_string(),
                instruction_style: InstructionStyle::ClaudeCode,
            },
            install_policy: InstallPolicy::CreateIfAbsent,
        }
    }

    /// Build a generic-agents-md target definition (stub).
    pub fn generic_agents_md(path: Option<PathBuf>) -> Self {
        let default_path = path.unwrap_or_else(|| PathBuf::from("agents.md"));
        IntegrationTarget {
            identifier: "generic-agents-md".to_string(),
            default_path,
            rendering_hints: RenderingHints {
                section_title: "Oversight Knowledge Base".to_string(),
                instruction_style: InstructionStyle::Generic,
            },
            install_policy: InstallPolicy::RequireExisting,
        }
    }

    /// Resolve the effective file path, allowing an explicit override.
    pub fn resolve_path(&self, path_override: Option<&Path>) -> PathBuf {
        match path_override {
            Some(p) => p.to_path_buf(),
            None => self.default_path.clone(),
        }
    }
}

/// Look up a target by identifier string.
pub fn resolve_target(identifier: &str) -> Result<IntegrationTarget> {
    match identifier {
        "claude-code" => Ok(IntegrationTarget::claude_code()),
        "generic-agents-md" => Ok(IntegrationTarget::generic_agents_md(None)),
        other => Err(Error::UnknownTarget(other.to_string())),
    }
}

/// Validate that a path is safe for writing.
///
/// Rejects paths that:
/// - Are empty
/// - Contain `..` path traversal components
/// - Contain null bytes
pub fn validate_target_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(Error::Integration("Target path is empty".to_string()));
    }

    let path_str = path.to_string_lossy();

    if path_str.contains('\0') {
        return Err(Error::Integration(
            "Target path contains null bytes".to_string(),
        ));
    }

    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return Err(Error::Integration(
                "Target path contains '..' traversal component".to_string(),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_code_target() {
        let target = IntegrationTarget::claude_code();
        assert_eq!(target.identifier, "claude-code");
        assert!(target.default_path.to_string_lossy().contains(".claude"));
        assert!(target.default_path.to_string_lossy().ends_with("CLAUDE.md"));
        assert_eq!(target.install_policy, InstallPolicy::CreateIfAbsent);
    }

    #[test]
    fn test_resolve_known_target() {
        let target = resolve_target("claude-code").unwrap();
        assert_eq!(target.identifier, "claude-code");
    }

    #[test]
    fn test_resolve_unknown_target() {
        let err = resolve_target("nonexistent").unwrap_err();
        assert!(err.to_string().contains("Unknown integration target"));
    }

    #[test]
    fn test_resolve_path_override() {
        let target = IntegrationTarget::claude_code();
        let custom = PathBuf::from("/custom/path/CLAUDE.md");
        let resolved = target.resolve_path(Some(&custom));
        assert_eq!(resolved, custom);
    }

    #[test]
    fn test_resolve_path_default() {
        let target = IntegrationTarget::claude_code();
        let resolved = target.resolve_path(None);
        assert_eq!(resolved, target.default_path);
    }

    #[test]
    fn test_validate_empty_path() {
        let err = validate_target_path(Path::new("")).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_validate_normal_path() {
        validate_target_path(Path::new("/home/user/.claude/CLAUDE.md")).unwrap();
    }

    #[test]
    fn test_validate_traversal_path() {
        let err = validate_target_path(Path::new("/home/user/../etc/passwd")).unwrap_err();
        assert!(err.to_string().contains("traversal"));
    }

    #[test]
    fn test_validate_relative_traversal() {
        let err = validate_target_path(Path::new("../../../etc/shadow")).unwrap_err();
        assert!(err.to_string().contains("traversal"));
    }
}
