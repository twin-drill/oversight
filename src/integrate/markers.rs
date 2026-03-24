use crate::error::{Error, Result};

/// Begin marker format: `<!-- oversight:begin target=<identifier> -->`
const BEGIN_PREFIX: &str = "<!-- oversight:begin target=";
const BEGIN_SUFFIX: &str = " -->";
/// End marker: `<!-- oversight:end -->`
const END_MARKER: &str = "<!-- oversight:end -->";

/// Represents the location of a managed block in file content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLocation {
    /// Byte offset of the start of the begin marker line (inclusive).
    pub start: usize,
    /// Byte offset of the end of the end marker line (exclusive, includes trailing newline if present).
    pub end: usize,
}

/// Format the begin marker for a given target identifier.
pub fn begin_marker(target: &str) -> String {
    format!("{BEGIN_PREFIX}{target}{BEGIN_SUFFIX}")
}

/// Format the end marker.
pub fn end_marker() -> String {
    END_MARKER.to_string()
}

/// Find the managed block for a given target in the content.
///
/// Returns `Ok(Some(location))` if exactly one matched pair is found.
/// Returns `Ok(None)` if no begin marker is found for this target.
/// Returns `Err` for malformed states (missing end marker, duplicate blocks).
pub fn find_block(content: &str, target: &str, file_path: &str) -> Result<Option<BlockLocation>> {
    let marker = begin_marker(target);
    let mut occurrences: Vec<usize> = Vec::new();

    for (idx, _) in content.match_indices(&marker) {
        occurrences.push(idx);
    }

    if occurrences.is_empty() {
        return Ok(None);
    }

    if occurrences.len() > 1 {
        return Err(Error::MalformedBlock {
            path: file_path.to_string(),
            detail: format!(
                "Found {} begin markers for target '{}'. Remove duplicates manually before retrying.",
                occurrences.len(),
                target
            ),
        });
    }

    let start = occurrences[0];

    // Find the end marker after the begin marker
    let search_from = start + marker.len();
    let end_pos = content[search_from..]
        .find(END_MARKER)
        .map(|pos| search_from + pos);

    match end_pos {
        Some(pos) => {
            // end is after the end marker line, including trailing newline
            let mut end = pos + END_MARKER.len();
            if content[end..].starts_with('\n') {
                end += 1;
            }
            Ok(Some(BlockLocation { start, end }))
        }
        None => Err(Error::MalformedBlock {
            path: file_path.to_string(),
            detail: format!(
                "Found begin marker for target '{}' but no matching end marker. \
                 Add '{}' after the managed section to repair.",
                target, END_MARKER
            ),
        }),
    }
}

/// Check if a managed block exists for the given target.
pub fn has_block(content: &str, target: &str) -> bool {
    let marker = begin_marker(target);
    content.contains(&marker)
}

/// Insert a managed block at the end of the content.
///
/// Ensures proper spacing (blank line before block if content is non-empty).
pub fn insert_block(content: &str, block: &str) -> String {
    if content.is_empty() {
        return block.to_string();
    }

    let mut result = content.to_string();
    // Ensure content ends with exactly one newline before adding the block
    if !result.ends_with('\n') {
        result.push('\n');
    }
    // Add a blank line separator
    result.push('\n');
    result.push_str(block);
    result
}

/// Replace the existing managed block with new content.
///
/// The block at `location` is replaced entirely with `new_block`.
pub fn replace_block(content: &str, location: &BlockLocation, new_block: &str) -> String {
    let mut result = String::with_capacity(content.len());
    result.push_str(&content[..location.start]);
    result.push_str(new_block);
    result.push_str(&content[location.end..]);
    result
}

/// Remove the managed block from content.
///
/// Also cleans up any extra blank lines left behind.
pub fn remove_block(content: &str, location: &BlockLocation) -> String {
    let before = &content[..location.start];
    let after = &content[location.end..];

    let mut result = String::with_capacity(content.len());
    result.push_str(before);
    result.push_str(after);

    // Clean up trailing whitespace at the join point:
    // If we end up with multiple consecutive blank lines, reduce to at most one.
    let cleaned = collapse_blank_lines(&result);
    cleaned.trim_end().to_string()
}

/// Collapse runs of 3+ consecutive newlines down to 2 (one blank line).
fn collapse_blank_lines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut newline_count = 0;

    for ch in s.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push(ch);
            }
        } else {
            newline_count = 0;
            result.push(ch);
        }
    }

    result
}

/// Wrap content with managed block markers.
pub fn wrap_block(target: &str, content: &str) -> String {
    let begin = begin_marker(target);
    let end = end_marker();
    let mut block = String::new();
    block.push_str(&begin);
    block.push('\n');
    block.push_str(content);
    if !content.ends_with('\n') {
        block.push('\n');
    }
    block.push_str(&end);
    block.push('\n');
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_begin_marker_format() {
        assert_eq!(
            begin_marker("claude-code"),
            "<!-- oversight:begin target=claude-code -->"
        );
    }

    #[test]
    fn test_end_marker_format() {
        assert_eq!(end_marker(), "<!-- oversight:end -->");
    }

    #[test]
    fn test_wrap_block() {
        let block = wrap_block("claude-code", "Hello\nWorld\n");
        assert!(block.starts_with("<!-- oversight:begin target=claude-code -->"));
        assert!(block.contains("Hello\nWorld\n"));
        assert!(block.ends_with("<!-- oversight:end -->\n"));
    }

    #[test]
    fn test_find_block_present() {
        let content = "Some header\n\n<!-- oversight:begin target=claude-code -->\nManaged\n<!-- oversight:end -->\n\nFooter\n";
        let loc = find_block(content, "claude-code", "test.md").unwrap().unwrap();
        let extracted = &content[loc.start..loc.end];
        assert!(extracted.contains("oversight:begin"));
        assert!(extracted.contains("oversight:end"));
        assert!(extracted.contains("Managed"));
    }

    #[test]
    fn test_find_block_absent() {
        let content = "No managed block here\n";
        let result = find_block(content, "claude-code", "test.md").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_find_block_missing_end() {
        let content = "<!-- oversight:begin target=claude-code -->\nManaged content\n";
        let err = find_block(content, "claude-code", "test.md").unwrap_err();
        assert!(err.to_string().contains("no matching end marker"));
    }

    #[test]
    fn test_find_block_duplicate() {
        let content = "<!-- oversight:begin target=claude-code -->\nA\n<!-- oversight:end -->\n<!-- oversight:begin target=claude-code -->\nB\n<!-- oversight:end -->\n";
        let err = find_block(content, "claude-code", "test.md").unwrap_err();
        assert!(err.to_string().contains("2 begin markers"));
    }

    #[test]
    fn test_has_block() {
        let content = "<!-- oversight:begin target=claude-code -->\nStuff\n<!-- oversight:end -->\n";
        assert!(has_block(content, "claude-code"));
        assert!(!has_block(content, "generic-agents-md"));
    }

    #[test]
    fn test_insert_block_empty_content() {
        let block = "<!-- oversight:begin target=claude-code -->\nHello\n<!-- oversight:end -->\n";
        let result = insert_block("", block);
        assert_eq!(result, block);
    }

    #[test]
    fn test_insert_block_existing_content() {
        let existing = "# My Config\n\nSome content here.\n";
        let block = "<!-- oversight:begin target=claude-code -->\nHello\n<!-- oversight:end -->\n";
        let result = insert_block(existing, block);
        assert!(result.starts_with("# My Config"));
        assert!(result.contains("\n\n<!-- oversight:begin"));
        assert!(result.ends_with("<!-- oversight:end -->\n"));
    }

    #[test]
    fn test_replace_block() {
        let content = "Header\n\n<!-- oversight:begin target=claude-code -->\nOld\n<!-- oversight:end -->\n\nFooter\n";
        let loc = find_block(content, "claude-code", "test.md").unwrap().unwrap();
        let new_block = "<!-- oversight:begin target=claude-code -->\nNew\n<!-- oversight:end -->\n";
        let result = replace_block(content, &loc, new_block);
        assert!(result.contains("New"));
        assert!(!result.contains("Old"));
        assert!(result.contains("Header"));
        assert!(result.contains("Footer"));
    }

    #[test]
    fn test_remove_block() {
        let content = "Header\n\n<!-- oversight:begin target=claude-code -->\nManaged\n<!-- oversight:end -->\n\nFooter\n";
        let loc = find_block(content, "claude-code", "test.md").unwrap().unwrap();
        let result = remove_block(content, &loc);
        assert!(!result.contains("oversight:begin"));
        assert!(!result.contains("Managed"));
        assert!(result.contains("Header"));
        assert!(result.contains("Footer"));
    }

    #[test]
    fn test_remove_block_only_managed() {
        let content = "<!-- oversight:begin target=claude-code -->\nManaged\n<!-- oversight:end -->\n";
        let loc = find_block(content, "claude-code", "test.md").unwrap().unwrap();
        let result = remove_block(content, &loc);
        assert!(result.is_empty() || result.chars().all(|c| c.is_whitespace()));
    }

    #[test]
    fn test_different_targets_dont_conflict() {
        let content = "<!-- oversight:begin target=claude-code -->\nA\n<!-- oversight:end -->\n\n<!-- oversight:begin target=generic-agents-md -->\nB\n<!-- oversight:end -->\n";
        let loc_cc = find_block(content, "claude-code", "test.md").unwrap().unwrap();
        let loc_ga = find_block(content, "generic-agents-md", "test.md").unwrap().unwrap();
        assert_ne!(loc_cc.start, loc_ga.start);
    }
}
