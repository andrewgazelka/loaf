mod helpers;

use helpers::{TestOverlay, init_test};
use loaf::db::ItemType;

#[test]
fn test_read_non_existent_file_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Try to read a file that doesn't exist
    let result = ctx.overlay.lookup(root_id, "nonexistent.txt");
    assert!(
        result.is_err(),
        "lookup of non-existent file should return error"
    );

    Ok(())
}

#[test]
fn test_lookup_non_existent_file_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Lookup should fail for non-existent file
    let result = ctx.overlay.lookup(root_id, "missing.txt");
    assert!(result.is_err(), "lookup should fail for non-existent file");

    Ok(())
}

#[test]
fn test_lookup_in_non_existent_directory_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create a file (not a directory)
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", ItemType::File, 0o644)?;

    // Try to lookup something inside the file (should fail)
    let result = ctx.overlay.lookup(file_id, "anything");
    assert!(
        result.is_err(),
        "lookup in non-directory should return error"
    );

    Ok(())
}

#[test]
fn test_remove_non_existent_inode_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;

    // Try to remove an inode that doesn't exist
    let result = ctx.overlay.remove(999999);
    assert!(
        result.is_err(),
        "remove of non-existent inode should return error"
    );

    Ok(())
}

#[test]
fn test_readlink_on_non_symlink_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create a regular file
    let file_id = ctx
        .overlay
        .create(root_id, "regular.txt", ItemType::File, 0o644)?;

    // Try to readlink on a regular file
    let result = ctx.overlay.readlink(file_id);
    assert!(
        result.is_err(),
        "readlink on non-symlink should return error"
    );

    Ok(())
}

#[test]
fn test_readlink_on_directory_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create a directory
    let dir_id = ctx
        .overlay
        .create(root_id, "dir", ItemType::Directory, 0o755)?;

    // Try to readlink on a directory
    let result = ctx.overlay.readlink(dir_id);
    assert!(result.is_err(), "readlink on directory should return error");

    Ok(())
}

#[test]
fn test_lookup_removed_file_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create and remove a file
    let file_id = ctx
        .overlay
        .create(root_id, "temp.txt", ItemType::File, 0o644)?;
    ctx.overlay.remove(file_id)?;

    // Lookup should fail after removal
    let result = ctx.overlay.lookup(root_id, "temp.txt");
    assert!(
        result.is_err(),
        "lookup of removed file should return error"
    );

    Ok(())
}

#[test]
fn test_getattr_on_whited_out_file_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;

    // Create a base file
    ctx.create_base_file("base.txt", b"content")?;

    let root_id = ctx.root_id();

    // Lookup the base file first to get its inode
    let file_id = ctx.overlay.lookup(root_id, "base.txt")?;

    // Remove it (creates whiteout)
    ctx.overlay.remove(file_id)?;

    // Try to getattr on the whited-out inode
    let result = ctx.overlay.getattr(file_id);
    assert!(
        result.is_err(),
        "getattr on whited-out file should return error"
    );

    Ok(())
}

#[test]
fn test_read_from_non_existent_inode_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;

    // Try to read from a very high inode that doesn't exist
    let mut buf = vec![0u8; 100];
    let result = ctx.overlay.read(999999, 0, &mut buf);
    assert!(
        result.is_err(),
        "read from non-existent inode should return error"
    );

    Ok(())
}

#[test]
fn test_write_to_non_existent_inode_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;

    // Try to write to a very high inode that doesn't exist
    let result = ctx.overlay.write(999999, 0, b"data");
    assert!(
        result.is_err(),
        "write to non-existent inode should return error"
    );

    Ok(())
}

#[test]
fn test_setattr_on_non_existent_inode_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;

    // Try to setattr on a very high inode that doesn't exist
    let result = ctx.overlay.setattr(999999, Some(0o644), None, None, None);
    assert!(
        result.is_err(),
        "setattr on non-existent inode should return error"
    );

    Ok(())
}

#[test]
fn test_rename_non_existent_file_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Try to rename a file that doesn't exist
    let result = ctx
        .overlay
        .rename(root_id, "nonexistent.txt", root_id, "new.txt");
    assert!(
        result.is_err(),
        "rename of non-existent file should return error"
    );

    Ok(())
}

#[test]
fn test_readdir_on_non_existent_directory_returns_error() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;

    // Try to readdir on a very high inode that doesn't exist
    let result = ctx.overlay.readdir(999999);
    assert!(
        result.is_err(),
        "readdir on non-existent directory should return error"
    );

    Ok(())
}
