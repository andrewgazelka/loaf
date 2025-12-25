mod helpers;

use helpers::{TestOverlay, init_test};
use loaf::db::ItemType;

#[test]
fn test_create_symlink_absolute_target() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let link_id = ctx
        .overlay
        .symlink(root_id, "absolute_link", "/usr/bin/ls")?;

    // Verify getattr shows symlink type
    let attrs = ctx.overlay.getattr(link_id)?;
    assert_eq!(attrs.item_type, ItemType::Symlink);

    // Verify readlink returns target
    let target = ctx.overlay.readlink(link_id)?;
    assert_eq!(target, "/usr/bin/ls");

    Ok(())
}

#[test]
fn test_create_symlink_relative_target() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let link_id = ctx
        .overlay
        .symlink(root_id, "relative_link", "../target/file.txt")?;

    let target = ctx.overlay.readlink(link_id)?;
    assert_eq!(target, "../target/file.txt");

    Ok(())
}

#[test]
fn test_readlink_returns_correct_target() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let target_path = "/some/complex/path/to/target";
    let link_id = ctx.overlay.symlink(root_id, "link", target_path)?;

    let retrieved_target = ctx.overlay.readlink(link_id)?;
    assert_eq!(
        retrieved_target, target_path,
        "readlink should return exact target"
    );

    Ok(())
}

#[test]
fn test_symlink_getattr_shows_symlink_type() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let link_id = ctx.overlay.symlink(root_id, "typecheck", "/target")?;

    let attrs = ctx.overlay.getattr(link_id)?;
    assert_eq!(attrs.item_type, ItemType::Symlink, "type should be symlink");

    Ok(())
}

#[test]
fn test_remove_symlink_creates_whiteout() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;

    // Create symlink in base
    use std::os::unix::fs::symlink as unix_symlink;
    unix_symlink("/target", ctx.base_path.join("base_link"))
        .expect("failed to create base symlink");

    let root_id = ctx.root_id();

    // Lookup and remove
    let link_id = ctx.overlay.lookup(root_id, "base_link")?;
    ctx.overlay.remove(link_id)?;

    // Verify it's gone
    let result = ctx.overlay.lookup(root_id, "base_link");
    assert!(result.is_err(), "removed symlink should not be found");

    Ok(())
}

#[test]
fn test_symlink_to_nonexistent_target_is_valid() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Symlinks to non-existent targets are valid
    let link_id = ctx
        .overlay
        .symlink(root_id, "dangling", "/does/not/exist")?;

    // Should be able to read the target even though it doesn't exist
    let target = ctx.overlay.readlink(link_id)?;
    assert_eq!(target, "/does/not/exist");

    Ok(())
}

#[test]
fn test_symlink_in_subdirectory() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let dir_id = ctx.overlay.mkdir(root_id, "subdir", 0o755)?;
    let link_id = ctx.overlay.symlink(dir_id, "nested_link", "/target/path")?;

    // Lookup via parent
    let found_id = ctx.overlay.lookup(dir_id, "nested_link")?;
    assert_eq!(found_id, link_id);

    // Verify target
    let target = ctx.overlay.readlink(link_id)?;
    assert_eq!(target, "/target/path");

    Ok(())
}
