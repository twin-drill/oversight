use oversight::integrate::markers;

#[test]
fn test_begin_marker_format() {
    let marker = markers::begin_marker("claude-code");
    assert_eq!(marker, "<!-- oversight:begin target=claude-code -->");
}

#[test]
fn test_end_marker_format() {
    assert_eq!(markers::end_marker(), "<!-- oversight:end -->");
}

#[test]
fn test_wrap_block_basic() {
    let block = markers::wrap_block("claude-code", "Some content\n");
    assert!(block.starts_with("<!-- oversight:begin target=claude-code -->"));
    assert!(block.contains("Some content"));
    assert!(block.ends_with("<!-- oversight:end -->\n"));
}

#[test]
fn test_find_block_in_existing_config() {
    let content = "\
# My Config

Some existing stuff.

<!-- oversight:begin target=claude-code -->
## Oversight KB
Current topics: gh-cli
<!-- oversight:end -->

More stuff below.
";
    let loc = markers::find_block(content, "claude-code", "test.md")
        .unwrap()
        .unwrap();
    let extracted = &content[loc.start..loc.end];
    assert!(extracted.contains("oversight:begin"));
    assert!(extracted.contains("Current topics: gh-cli"));
    assert!(extracted.contains("oversight:end"));
}

#[test]
fn test_find_block_absent() {
    let content = "# Just a normal file\nNo managed block here.\n";
    let result = markers::find_block(content, "claude-code", "test.md").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_find_block_missing_end_marker_error() {
    let content = "<!-- oversight:begin target=claude-code -->\nOrphan block\n";
    let err = markers::find_block(content, "claude-code", "test.md").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("no matching end marker"), "Got: {msg}");
}

#[test]
fn test_find_block_duplicate_begin_markers_error() {
    let content = "\
<!-- oversight:begin target=claude-code -->
Block A
<!-- oversight:end -->
<!-- oversight:begin target=claude-code -->
Block B
<!-- oversight:end -->
";
    let err = markers::find_block(content, "claude-code", "test.md").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("2 begin markers"), "Got: {msg}");
}

#[test]
fn test_has_block_true() {
    let content = "<!-- oversight:begin target=claude-code -->\nStuff\n<!-- oversight:end -->\n";
    assert!(markers::has_block(content, "claude-code"));
}

#[test]
fn test_has_block_false() {
    let content = "No block here\n";
    assert!(!markers::has_block(content, "claude-code"));
}

#[test]
fn test_has_block_different_target() {
    let content = "<!-- oversight:begin target=generic-agents-md -->\nStuff\n<!-- oversight:end -->\n";
    assert!(!markers::has_block(content, "claude-code"));
    assert!(markers::has_block(content, "generic-agents-md"));
}

#[test]
fn test_insert_block_into_empty() {
    let block = markers::wrap_block("claude-code", "Content\n");
    let result = markers::insert_block("", &block);
    assert_eq!(result, block);
}

#[test]
fn test_insert_block_into_existing_content() {
    let existing = "# Header\n\nExisting content.\n";
    let block = markers::wrap_block("claude-code", "Managed\n");
    let result = markers::insert_block(existing, &block);

    // Should start with original content
    assert!(result.starts_with("# Header"));
    // Should have a blank line before managed block
    assert!(result.contains("\n\n<!-- oversight:begin"));
    // Should end with end marker
    assert!(result.trim_end().ends_with("<!-- oversight:end -->"));
}

#[test]
fn test_replace_block_preserves_surrounding() {
    let content = "\
# Header

<!-- oversight:begin target=claude-code -->
Old managed content
<!-- oversight:end -->

# Footer
";
    let loc = markers::find_block(content, "claude-code", "test.md")
        .unwrap()
        .unwrap();
    let new_block = markers::wrap_block("claude-code", "New managed content\n");
    let result = markers::replace_block(content, &loc, &new_block);

    assert!(result.contains("# Header"));
    assert!(result.contains("New managed content"));
    assert!(!result.contains("Old managed content"));
    assert!(result.contains("# Footer"));
}

#[test]
fn test_remove_block_preserves_surrounding() {
    let content = "\
# Header

Some content.

<!-- oversight:begin target=claude-code -->
Managed content
<!-- oversight:end -->

# Footer
";
    let loc = markers::find_block(content, "claude-code", "test.md")
        .unwrap()
        .unwrap();
    let result = markers::remove_block(content, &loc);

    assert!(result.contains("# Header"));
    assert!(result.contains("Some content."));
    assert!(!result.contains("Managed content"));
    assert!(!result.contains("oversight:begin"));
    assert!(result.contains("# Footer"));
}

#[test]
fn test_remove_block_only_managed() {
    let content = markers::wrap_block("claude-code", "Managed\n");
    let loc = markers::find_block(&content, "claude-code", "test.md")
        .unwrap()
        .unwrap();
    let result = markers::remove_block(&content, &loc);
    assert!(
        result.trim().is_empty(),
        "Expected empty after removing only block, got: '{result}'"
    );
}

#[test]
fn test_multiple_targets_independent() {
    let block_a = markers::wrap_block("claude-code", "A content\n");
    let block_b = markers::wrap_block("generic-agents-md", "B content\n");
    let content = format!("{block_a}\n{block_b}");

    // Should find each independently
    let loc_a = markers::find_block(&content, "claude-code", "test.md")
        .unwrap()
        .unwrap();
    let loc_b = markers::find_block(&content, "generic-agents-md", "test.md")
        .unwrap()
        .unwrap();

    assert_ne!(loc_a.start, loc_b.start);

    // Remove A should leave B intact
    let after_remove_a = markers::remove_block(&content, &loc_a);
    assert!(!after_remove_a.contains("A content"));
    assert!(after_remove_a.contains("B content"));
    assert!(after_remove_a.contains("generic-agents-md"));
}
