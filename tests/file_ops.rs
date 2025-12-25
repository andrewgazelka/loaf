mod helpers;

use helpers::{TestOverlay, init_test};
use loaf::db::ItemType;

#[test]
fn test_create_file_and_verify_exists() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "test.txt", ItemType::File, 0o644)?;

    // Verify lookup works
    let lookup_id = ctx.overlay.lookup(root_id, "test.txt")?;
    assert_eq!(lookup_id, file_id, "lookup should return same inode");

    // Verify getattr shows correct type
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.item_type, ItemType::File);
    assert_eq!(attrs.mode, 0o644);

    Ok(())
}

#[test]
fn test_write_and_read_data() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "data.txt", ItemType::File, 0o644)?;

    // Write data
    let data = b"hello world from overlay";
    let n = ctx.overlay.write(file_id, 0, data)?;
    assert_eq!(n, data.len(), "write should return bytes written");

    // Read back
    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(n, data.len(), "read should return correct length");
    assert_eq!(&buf[..n], data, "read data should match written data");

    Ok(())
}

#[test]
fn test_append_data_at_offset() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "append.txt", ItemType::File, 0o644)?;

    // Write initial data
    ctx.overlay.write(file_id, 0, b"hello")?;

    // Append at offset 5
    ctx.overlay.write(file_id, 5, b" world")?;

    // Read full content
    let mut buf = vec![0u8; 50];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(
        &buf[..n],
        b"hello world",
        "appended data should be at correct offset"
    );

    Ok(())
}

#[test]
fn test_truncate_file() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "truncate.txt", ItemType::File, 0o644)?;

    // Write initial data
    ctx.overlay.write(file_id, 0, b"hello world")?;

    // Truncate to 5 bytes
    ctx.overlay.setattr(file_id, None, Some(5), None, None)?;

    // Verify size
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, 5);

    // Verify content
    let mut buf = vec![0u8; 10];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(n, 5);
    assert_eq!(&buf[..n], b"hello");

    Ok(())
}

#[test]
fn test_overwrite_existing_file() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "overwrite.txt", ItemType::File, 0o644)?;

    // Write initial data
    ctx.overlay.write(file_id, 0, b"first version")?;

    // Overwrite from start
    ctx.overlay.write(file_id, 0, b"second")?;

    // Read back - should have "second version" (partial overwrite)
    let mut buf = vec![0u8; 50];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(
        &buf[..n],
        b"second version",
        "partial overwrite should work"
    );

    Ok(())
}

#[test]
fn test_read_at_various_offsets() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "offsets.txt", ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"0123456789")?;

    // Read from offset 5
    let mut buf = vec![0u8; 5];
    let n = ctx.overlay.read(file_id, 5, &mut buf)?;
    assert_eq!(n, 5);
    assert_eq!(&buf[..n], b"56789");

    // Read from offset 8
    let mut buf = vec![0u8; 10];
    let n = ctx.overlay.read(file_id, 8, &mut buf)?;
    assert_eq!(n, 2);
    assert_eq!(&buf[..n], b"89");

    Ok(())
}

#[test]
fn test_read_beyond_eof_returns_zero() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "eof.txt", ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"short")?;

    // Read beyond EOF
    let mut buf = vec![0u8; 10];
    let n = ctx.overlay.read(file_id, 100, &mut buf)?;
    assert_eq!(n, 0, "read beyond EOF should return 0 bytes");

    Ok(())
}
