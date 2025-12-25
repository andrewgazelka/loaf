use color_eyre::eyre::WrapErr as _;

mod helpers;

#[test]
fn test_setattr_mode_changes_permissions() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file with initial mode
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", loaf::db::ItemType::File, 0o644)?;

    // Verify initial mode
    let attrs_before = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs_before.mode, 0o644);

    // Change mode
    ctx.overlay
        .setattr(file_id, Some(0o600), None, None, None)?;

    // Verify mode changed
    let attrs_after = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs_after.mode, 0o600);

    Ok(())
}

#[test]
fn test_setattr_size_truncates_file() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file with content
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"hello world")?;

    // Verify initial size
    let attrs_before = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs_before.size, 11);

    // Truncate to 5 bytes
    ctx.overlay.setattr(file_id, None, Some(5), None, None)?;

    // Verify size changed
    let attrs_after = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs_after.size, 5);

    // Verify content is truncated
    let mut buf = vec![0u8; 20];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(n, 5);
    assert_eq!(&buf[..5], b"hello");

    Ok(())
}

#[test]
fn test_setattr_size_extends_file_with_zeros() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file with small content
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"hello")?;

    // Extend to 10 bytes
    ctx.overlay.setattr(file_id, None, Some(10), None, None)?;

    // Verify size
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, 10);

    // Verify content (original + zeros)
    let mut buf = vec![0u8; 20];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(n, 10);
    assert_eq!(&buf[..5], b"hello");
    assert_eq!(&buf[5..10], &[0, 0, 0, 0, 0]);

    Ok(())
}

#[test]
fn test_setattr_atime_mtime_updates_timestamps() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", loaf::db::ItemType::File, 0o644)?;

    // Set specific timestamps
    let atime = (1000i64, 2000i64);
    let mtime = (3000i64, 4000i64);
    ctx.overlay
        .setattr(file_id, None, None, Some(atime), Some(mtime))?;

    // Verify timestamps
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.atime_sec, 1000);
    assert_eq!(attrs.atime_nsec, 2000);
    assert_eq!(attrs.mtime_sec, 3000);
    assert_eq!(attrs.mtime_nsec, 4000);

    Ok(())
}

#[test]
fn test_setattr_partial_time_update() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", loaf::db::ItemType::File, 0o644)?;

    // Set initial timestamps
    ctx.overlay
        .setattr(file_id, None, None, Some((1000, 1000)), Some((2000, 2000)))?;

    // Update only atime
    ctx.overlay
        .setattr(file_id, None, None, Some((5000, 5000)), None)?;

    // Verify atime changed, mtime unchanged
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.atime_sec, 5000);
    assert_eq!(attrs.atime_nsec, 5000);
    assert_eq!(attrs.mtime_sec, 2000);
    assert_eq!(attrs.mtime_nsec, 2000);

    Ok(())
}

#[test]
fn test_time_only_update_does_not_trigger_copy_on_write() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    // Create file in base
    ctx.create_base_file("base_file.txt", b"base content")?;

    let root_id = ctx.root_id();
    let file_id = ctx.overlay.lookup(root_id, "base_file.txt")?;

    // Update only atime/mtime (should NOT trigger copy-on-write)
    ctx.overlay
        .setattr(file_id, None, None, Some((1000, 0)), Some((2000, 0)))?;

    // Verify file is NOT in overlay database (no copy-on-write)
    let inodes = ctx.overlay.get_all_inodes()?;
    let paths: Vec<_> = inodes.iter().map(|(p, _)| p.as_str()).collect();
    assert!(
        !paths.contains(&"/base_file.txt"),
        "time-only update should not copy file to overlay"
    );

    Ok(())
}

#[test]
fn test_mode_change_triggers_copy_on_write() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    // Create file in base
    ctx.create_base_file("base_file.txt", b"base content")?;

    let root_id = ctx.root_id();
    let file_id = ctx.overlay.lookup(root_id, "base_file.txt")?;

    // Change mode (SHOULD trigger copy-on-write)
    ctx.overlay
        .setattr(file_id, Some(0o600), None, None, None)?;

    // Verify file IS in overlay database
    let inodes = ctx.overlay.get_all_inodes()?;
    let paths: Vec<_> = inodes.iter().map(|(p, _)| p.as_str()).collect();
    assert!(
        paths.contains(&"/base_file.txt"),
        "mode change should copy file to overlay"
    );

    // Verify base file is unchanged
    let base_content = std::fs::read(ctx.base_path.join("base_file.txt"))?;
    assert_eq!(&base_content, b"base content");

    Ok(())
}

#[test]
fn test_size_change_triggers_copy_on_write() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    // Create file in base
    ctx.create_base_file("base_file.txt", b"base content")?;

    let root_id = ctx.root_id();
    let file_id = ctx.overlay.lookup(root_id, "base_file.txt")?;

    // Truncate (SHOULD trigger copy-on-write)
    ctx.overlay.setattr(file_id, None, Some(4), None, None)?;

    // Verify file IS in overlay database
    let inodes = ctx.overlay.get_all_inodes()?;
    let paths: Vec<_> = inodes.iter().map(|(p, _)| p.as_str()).collect();
    assert!(
        paths.contains(&"/base_file.txt"),
        "size change should copy file to overlay"
    );

    // Verify truncation in overlay
    let mut buf = vec![0u8; 20];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(n, 4);
    assert_eq!(&buf[..4], b"base");

    // Verify base file is unchanged
    let base_content = std::fs::read(ctx.base_path.join("base_file.txt"))?;
    assert_eq!(&base_content, b"base content");

    Ok(())
}

#[test]
fn test_getattr_returns_correct_values() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file
    let file_id = ctx
        .overlay
        .create(root_id, "test.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"hello")?;

    // Set attributes
    ctx.overlay.setattr(
        file_id,
        Some(0o755),
        Some(10),
        Some((1000, 0)),
        Some((2000, 0)),
    )?;

    // Verify all attributes
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.file_id, file_id);
    assert_eq!(attrs.item_type, loaf::db::ItemType::File);
    assert_eq!(attrs.mode, 0o755);
    assert_eq!(attrs.size, 10);
    assert_eq!(attrs.atime_sec, 1000);
    assert_eq!(attrs.mtime_sec, 2000);

    Ok(())
}

#[test]
fn test_setattr_multiple_attributes_at_once() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"initial content")?;

    // Set multiple attributes at once
    ctx.overlay.setattr(
        file_id,
        Some(0o600),     // mode
        Some(7),         // size (truncate)
        Some((5000, 0)), // atime
        Some((6000, 0)), // mtime
    )?;

    // Verify all changes
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.mode, 0o600);
    assert_eq!(attrs.size, 7);
    assert_eq!(attrs.atime_sec, 5000);
    assert_eq!(attrs.mtime_sec, 6000);

    // Verify content truncation
    let mut buf = vec![0u8; 20];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(n, 7);
    assert_eq!(&buf[..7], b"initial");

    Ok(())
}

#[test]
fn test_setattr_on_directory() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create directory
    let dir_id = ctx.overlay.mkdir(root_id, "testdir", 0o755)?;

    // Change directory mode
    ctx.overlay.setattr(dir_id, Some(0o700), None, None, None)?;

    // Verify mode changed
    let attrs = ctx.overlay.getattr(dir_id)?;
    assert_eq!(attrs.mode, 0o700);
    assert_eq!(attrs.item_type, loaf::db::ItemType::Directory);

    Ok(())
}

#[test]
fn test_setattr_zero_size_empties_file() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file with content
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"lots of content here")?;

    // Truncate to zero
    ctx.overlay.setattr(file_id, None, Some(0), None, None)?;

    // Verify empty
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, 0);

    let mut buf = vec![0u8; 20];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(n, 0);

    Ok(())
}

#[test]
fn test_setattr_preserves_content_on_mode_change() -> color_eyre::Result<()> {
    helpers::init_test();
    let ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file with content
    let file_id = ctx
        .overlay
        .create(root_id, "file.txt", loaf::db::ItemType::File, 0o644)?;
    let content = b"important data";
    ctx.overlay.write(file_id, 0, content)?;

    // Change only mode
    ctx.overlay
        .setattr(file_id, Some(0o600), None, None, None)?;

    // Verify content unchanged
    let mut buf = vec![0u8; 20];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], content);

    Ok(())
}
