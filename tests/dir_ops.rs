mod helpers;

use helpers::{TestOverlay, init_test};
use loaf::db::ItemType;

#[test]
fn test_mkdir_creates_directory() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let dir_id = ctx.overlay.mkdir(root_id, "testdir", 0o755)?;

    // Verify type
    let attrs = ctx.overlay.getattr(dir_id)?;
    assert_eq!(attrs.item_type, ItemType::Directory);
    assert_eq!(attrs.mode, 0o755);

    // Verify lookup
    let lookup_id = ctx.overlay.lookup(root_id, "testdir")?;
    assert_eq!(lookup_id, dir_id);

    Ok(())
}

#[test]
fn test_nested_directory_creation() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create parent directory
    let parent_id = ctx.overlay.mkdir(root_id, "parent", 0o755)?;

    // Create child directory
    let child_id = ctx.overlay.mkdir(parent_id, "child", 0o755)?;

    // Create grandchild directory
    let grandchild_id = ctx.overlay.mkdir(child_id, "grandchild", 0o755)?;

    // Verify lookup through hierarchy
    let p = ctx.overlay.lookup(root_id, "parent")?;
    let c = ctx.overlay.lookup(p, "child")?;
    let g = ctx.overlay.lookup(c, "grandchild")?;
    assert_eq!(g, grandchild_id);

    Ok(())
}

#[test]
fn test_readdir_lists_children() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create multiple entries
    ctx.overlay
        .create(root_id, "file1.txt", ItemType::File, 0o644)?;
    ctx.overlay
        .create(root_id, "file2.txt", ItemType::File, 0o644)?;
    ctx.overlay.mkdir(root_id, "dir1", 0o755)?;
    ctx.overlay.symlink(root_id, "link1", "/target")?;

    // List directory
    let entries = ctx.overlay.readdir(root_id)?;
    assert_eq!(entries.len(), 4, "should have 4 entries");

    let names: std::collections::HashSet<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains("file1.txt"));
    assert!(names.contains("file2.txt"));
    assert!(names.contains("dir1"));
    assert!(names.contains("link1"));

    Ok(())
}

#[test]
fn test_readdir_merges_overlay_and_base() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;

    // Create files in base
    ctx.create_base_file("base1.txt", b"base content 1")?;
    ctx.create_base_file("base2.txt", b"base content 2")?;

    // Create files in overlay
    let root_id = ctx.root_id();
    ctx.overlay
        .create(root_id, "overlay1.txt", ItemType::File, 0o644)?;
    ctx.overlay
        .create(root_id, "overlay2.txt", ItemType::File, 0o644)?;

    // List directory
    let entries = ctx.overlay.readdir(root_id)?;
    assert_eq!(entries.len(), 4, "should have 2 base + 2 overlay entries");

    let names: std::collections::HashSet<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains("base1.txt"));
    assert!(names.contains("base2.txt"));
    assert!(names.contains("overlay1.txt"));
    assert!(names.contains("overlay2.txt"));

    Ok(())
}

#[test]
fn test_lookup_in_subdirectory() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let dir_id = ctx.overlay.mkdir(root_id, "subdir", 0o755)?;
    let file_id = ctx
        .overlay
        .create(dir_id, "nested.txt", ItemType::File, 0o644)?;

    // Lookup via subdirectory
    let found_id = ctx.overlay.lookup(dir_id, "nested.txt")?;
    assert_eq!(found_id, file_id);

    Ok(())
}

#[test]
fn test_remove_empty_directory() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let dir_id = ctx.overlay.mkdir(root_id, "emptydir", 0o755)?;

    // Remove directory
    ctx.overlay.remove(dir_id)?;

    // Verify it's gone
    let result = ctx.overlay.lookup(root_id, "emptydir");
    assert!(result.is_err(), "removed directory should not be found");

    Ok(())
}

#[test]
fn test_remove_directory_preserves_siblings() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let dir1_id = ctx.overlay.mkdir(root_id, "dir1", 0o755)?;
    let _file_id = ctx
        .overlay
        .create(root_id, "file1.txt", ItemType::File, 0o644)?;

    // Remove dir1
    ctx.overlay.remove(dir1_id)?;

    // Verify file1.txt still exists
    let file_lookup = ctx.overlay.lookup(root_id, "file1.txt");
    assert!(file_lookup.is_ok(), "sibling file should still exist");

    Ok(())
}
