use color_eyre::eyre::WrapErr as _;

mod helpers;

#[test]
fn test_rename_file_same_directory() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file
    let file_id = ctx
        .overlay
        .create(root_id, "old.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"test content")?;

    // Rename within same directory
    ctx.overlay.rename(root_id, "old.txt", root_id, "new.txt")?;

    // Verify new name exists
    let new_id = ctx.overlay.lookup(root_id, "new.txt")?;
    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(new_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"test content");

    // Verify old name is gone
    assert!(
        ctx.overlay.lookup(root_id, "old.txt").is_err(),
        "old.txt should not exist after rename"
    );

    Ok(())
}

#[test]
fn test_rename_file_to_different_directory() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create directories
    let src_dir_id = ctx.overlay.mkdir(root_id, "src", 0o755)?;
    let dst_dir_id = ctx.overlay.mkdir(root_id, "dst", 0o755)?;

    // Create file in src
    let file_id = ctx
        .overlay
        .create(src_dir_id, "file.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"moving content")?;

    // Rename to different directory
    ctx.overlay
        .rename(src_dir_id, "file.txt", dst_dir_id, "moved.txt")?;

    // Verify file exists in destination
    let moved_id = ctx.overlay.lookup(dst_dir_id, "moved.txt")?;
    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(moved_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"moving content");

    // Verify file is gone from source
    assert!(ctx.overlay.lookup(src_dir_id, "file.txt").is_err());

    Ok(())
}

#[test]
fn test_rename_preserves_file_content() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file with substantial content
    let file_id = ctx
        .overlay
        .create(root_id, "original.txt", loaf::db::ItemType::File, 0o644)?;
    let content =
        b"This is a longer piece of content that should be preserved across rename operations.";
    ctx.overlay.write(file_id, 0, content)?;

    // Rename
    ctx.overlay
        .rename(root_id, "original.txt", root_id, "renamed.txt")?;

    // Verify content is identical
    let renamed_id = ctx.overlay.lookup(root_id, "renamed.txt")?;
    let mut buf = vec![0u8; 200];
    let n = ctx.overlay.read(renamed_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], content);

    Ok(())
}

#[test]
fn test_rename_directory_with_children() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create directory with files
    let old_dir_id = ctx.overlay.mkdir(root_id, "olddir", 0o755)?;
    let file1_id = ctx
        .overlay
        .create(old_dir_id, "file1.txt", loaf::db::ItemType::File, 0o644)?;
    let file2_id = ctx
        .overlay
        .create(old_dir_id, "file2.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file1_id, 0, b"content1")?;
    ctx.overlay.write(file2_id, 0, b"content2")?;

    // Create subdirectory
    let subdir_id = ctx.overlay.mkdir(old_dir_id, "subdir", 0o755)?;
    let file3_id = ctx
        .overlay
        .create(subdir_id, "file3.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file3_id, 0, b"content3")?;

    // Rename directory
    ctx.overlay.rename(root_id, "olddir", root_id, "newdir")?;

    // Verify new directory exists
    let new_dir_id = ctx.overlay.lookup(root_id, "newdir")?;

    // Verify children exist
    let new_file1_id = ctx.overlay.lookup(new_dir_id, "file1.txt")?;
    let new_file2_id = ctx.overlay.lookup(new_dir_id, "file2.txt")?;

    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(new_file1_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"content1");

    let n = ctx.overlay.read(new_file2_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"content2");

    // Verify subdirectory and its children
    let new_subdir_id = ctx.overlay.lookup(new_dir_id, "subdir")?;
    let new_file3_id = ctx.overlay.lookup(new_subdir_id, "file3.txt")?;
    let n = ctx.overlay.read(new_file3_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"content3");

    // Verify old directory is gone
    assert!(ctx.overlay.lookup(root_id, "olddir").is_err());

    Ok(())
}

#[test]
fn test_rename_updates_descendant_paths() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create nested structure
    let dir_id = ctx.overlay.mkdir(root_id, "parent", 0o755)?;
    let child_id = ctx.overlay.mkdir(dir_id, "child", 0o755)?;
    let file_id = ctx
        .overlay
        .create(child_id, "deep.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"deep content")?;

    // Rename parent directory
    ctx.overlay
        .rename(root_id, "parent", root_id, "renamed_parent")?;

    // Verify entire tree is accessible via new path
    let new_parent_id = ctx.overlay.lookup(root_id, "renamed_parent")?;
    let new_child_id = ctx.overlay.lookup(new_parent_id, "child")?;
    let new_file_id = ctx.overlay.lookup(new_child_id, "deep.txt")?;

    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(new_file_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"deep content");

    Ok(())
}

#[test]
fn test_rename_over_existing_file() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create source file
    let src_id = ctx
        .overlay
        .create(root_id, "source.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(src_id, 0, b"source content")?;

    // Create destination file (to be replaced)
    let dst_id = ctx
        .overlay
        .create(root_id, "dest.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(dst_id, 0, b"dest content")?;

    // Rename source over destination
    ctx.overlay
        .rename(root_id, "source.txt", root_id, "dest.txt")?;

    // Verify destination has source content
    let final_id = ctx.overlay.lookup(root_id, "dest.txt")?;
    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(final_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"source content");

    // Verify source is gone
    assert!(ctx.overlay.lookup(root_id, "source.txt").is_err());

    Ok(())
}

#[test]
fn test_old_path_not_accessible_after_rename() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file
    let file_id = ctx
        .overlay
        .create(root_id, "before.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"data")?;

    // Rename
    ctx.overlay
        .rename(root_id, "before.txt", root_id, "after.txt")?;

    // Verify old path fails
    let result = ctx.overlay.lookup(root_id, "before.txt");
    assert!(result.is_err(), "old path should not be accessible");

    // Verify new path works
    let new_id = ctx.overlay.lookup(root_id, "after.txt")?;
    assert!(new_id > 0);

    Ok(())
}

#[test]
fn test_rename_symlink() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create symlink
    let link_id = ctx.overlay.symlink(root_id, "old_link", "/target/path")?;

    // Verify original symlink
    let target = ctx.overlay.readlink(link_id)?;
    assert_eq!(target, "/target/path");

    // Rename symlink
    ctx.overlay
        .rename(root_id, "old_link", root_id, "new_link")?;

    // Verify renamed symlink exists and points to same target
    let new_link_id = ctx.overlay.lookup(root_id, "new_link")?;
    let new_target = ctx.overlay.readlink(new_link_id)?;
    assert_eq!(new_target, "/target/path");

    // Verify old link is gone
    assert!(ctx.overlay.lookup(root_id, "old_link").is_err());

    Ok(())
}

#[test]
fn test_rename_from_base_to_overlay() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    // Create file in base
    ctx.create_base_file("base_file.txt", b"base content")?;

    let root_id = ctx.root_id();

    // Rename file from base (triggers copy-on-write)
    ctx.overlay
        .rename(root_id, "base_file.txt", root_id, "overlay_file.txt")?;

    // Verify renamed file exists in overlay
    let overlay_id = ctx.overlay.lookup(root_id, "overlay_file.txt")?;
    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(overlay_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"base content");

    // Verify original file in base is unchanged
    let base_content = std::fs::read(ctx.base_path.join("base_file.txt"))?;
    assert_eq!(&base_content, b"base content");

    // Verify old name not accessible via overlay
    assert!(ctx.overlay.lookup(root_id, "base_file.txt").is_err());

    Ok(())
}

#[test]
fn test_rename_preserves_mode() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file with specific mode
    let file_id = ctx
        .overlay
        .create(root_id, "executable.sh", loaf::db::ItemType::File, 0o755)?;
    ctx.overlay.write(file_id, 0, b"#!/bin/sh\necho hello")?;

    // Verify mode before rename
    let attrs_before = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs_before.mode, 0o755);

    // Rename
    ctx.overlay
        .rename(root_id, "executable.sh", root_id, "renamed.sh")?;

    // Verify mode after rename
    let renamed_id = ctx.overlay.lookup(root_id, "renamed.sh")?;
    let attrs_after = ctx.overlay.getattr(renamed_id)?;
    assert_eq!(attrs_after.mode, 0o755);

    Ok(())
}
