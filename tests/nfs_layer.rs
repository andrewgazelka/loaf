mod helpers;

use helpers::{TestOverlay, init_test};
use loaf::db::ItemType;
use loaf::nfs::NfsOverlay;
use nfsserve::vfs::{NFSFileSystem, VFSCapabilities};

#[tokio::test]
async fn test_nfs_overlay_capabilities() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);

    let caps = nfs.capabilities();
    assert_eq!(
        caps,
        VFSCapabilities::ReadWrite,
        "NFS overlay should have ReadWrite capabilities"
    );

    Ok(())
}

#[tokio::test]
async fn test_nfs_overlay_root_dir() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);

    let root = nfs.root_dir();
    assert_eq!(root, 1, "root directory should be inode 1");

    Ok(())
}

#[tokio::test]
async fn test_nfs_lookup_operation() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create a file in overlay
    let file_id = ctx
        .overlay
        .create(root_id, "test.txt", ItemType::File, 0o644)?;

    // Wrap in NFS layer
    let nfs = NfsOverlay::new(ctx.overlay);

    // Lookup through NFS
    let lookup_id = nfs.lookup(root_id, b"test.txt").await?;
    assert_eq!(lookup_id, file_id, "NFS lookup should return correct inode");

    Ok(())
}

#[tokio::test]
async fn test_nfs_lookup_non_existent_returns_error() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);
    let root_id = nfs.root_dir();

    // Lookup non-existent file
    let result = nfs.lookup(root_id, b"nonexistent.txt").await;
    assert!(
        result.is_err(),
        "NFS lookup of non-existent file should return error"
    );

    Ok(())
}

#[tokio::test]
async fn test_nfs_getattr_operation() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create a file
    let file_id = ctx
        .overlay
        .create(root_id, "attrs.txt", ItemType::File, 0o644)?;

    let nfs = NfsOverlay::new(ctx.overlay);

    // Get attributes through NFS
    let attrs = nfs.getattr(file_id).await?;

    // Verify type (NF3REG = regular file = 1)
    assert_eq!(attrs.ftype as u32, 1, "should be regular file type");

    // Verify mode includes file type bits (S_IFREG | 0o644)
    assert!(
        attrs.mode & 0o170000 != 0,
        "mode should include file type bits"
    );

    Ok(())
}

#[tokio::test]
async fn test_nfs_create_and_write_operations() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);
    let root_id = nfs.root_dir();

    // Create file through NFS
    let (file_id, attrs) = nfs.create(root_id, b"newfile.txt").await?;

    // Verify it's a file
    assert_eq!(attrs.ftype as u32, 1, "created item should be regular file");

    // Write data through NFS
    let data = b"hello from NFS";
    let write_result = nfs.write(file_id, 0, data).await?;
    assert_eq!(
        write_result.count,
        data.len() as u32,
        "write should return bytes written"
    );

    Ok(())
}

#[tokio::test]
async fn test_nfs_read_operation() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create and write data through overlay
    let file_id = ctx
        .overlay
        .create(root_id, "readable.txt", ItemType::File, 0o644)?;
    let data = b"test content";
    ctx.overlay.write(file_id, 0, data)?;

    let nfs = NfsOverlay::new(ctx.overlay);

    // Read through NFS
    let read_result = nfs.read(file_id, 0, 100).await?;

    assert_eq!(
        read_result.data.len(),
        data.len(),
        "read should return correct length"
    );
    assert_eq!(
        &read_result.data[..],
        data,
        "read data should match written data"
    );

    Ok(())
}

#[tokio::test]
async fn test_nfs_mkdir_operation() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);
    let root_id = nfs.root_dir();

    // Create directory through NFS
    let (dir_id, attrs) = nfs.mkdir(root_id, b"newdir").await?;

    // Verify it's a directory (NF3DIR = 2)
    assert_eq!(attrs.ftype as u32, 2, "created item should be directory");

    // Verify we can lookup the directory
    let lookup_id = nfs.lookup(root_id, b"newdir").await?;
    assert_eq!(
        lookup_id, dir_id,
        "lookup should return same directory inode"
    );

    Ok(())
}

#[tokio::test]
async fn test_nfs_remove_operation() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);
    let root_id = nfs.root_dir();

    // Create a file
    let (file_id, _) = nfs.create(root_id, b"deleteme.txt").await?;

    // Verify it exists
    let lookup_id = nfs.lookup(root_id, b"deleteme.txt").await?;
    assert_eq!(lookup_id, file_id, "file should exist before removal");

    // Remove it
    nfs.remove(root_id, b"deleteme.txt").await?;

    // Verify it's gone
    let result = nfs.lookup(root_id, b"deleteme.txt").await;
    assert!(result.is_err(), "file should not exist after removal");

    Ok(())
}

#[tokio::test]
async fn test_nfs_readdir_operation() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);
    let root_id = nfs.root_dir();

    // Create some files and directories
    nfs.create(root_id, b"file1.txt").await?;
    nfs.create(root_id, b"file2.txt").await?;
    nfs.mkdir(root_id, b"dir1").await?;

    // Read directory
    let entries = nfs.readdir(root_id, 0).await?;

    // Should have at least our 3 entries (may have . and ..)
    let entry_names: std::collections::HashSet<_> = entries
        .entries
        .iter()
        .map(|e| String::from_utf8_lossy(&e.name).to_string())
        .collect();

    assert!(
        entry_names.contains("file1.txt"),
        "readdir should include file1.txt"
    );
    assert!(
        entry_names.contains("file2.txt"),
        "readdir should include file2.txt"
    );
    assert!(entry_names.contains("dir1"), "readdir should include dir1");

    Ok(())
}

#[tokio::test]
async fn test_nfs_rename_operation() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);
    let root_id = nfs.root_dir();

    // Create a file
    let (file_id, _) = nfs.create(root_id, b"oldname.txt").await?;

    // Rename it
    nfs.rename(root_id, b"oldname.txt", root_id, b"newname.txt")
        .await?;

    // Old name should not exist
    let result = nfs.lookup(root_id, b"oldname.txt").await;
    assert!(result.is_err(), "old name should not exist after rename");

    // New name should exist and point to same inode
    let new_id = nfs.lookup(root_id, b"newname.txt").await?;
    assert_eq!(
        new_id, file_id,
        "renamed file should have same inode as original"
    );

    Ok(())
}

#[tokio::test]
async fn test_nfs_symlink_operation() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);
    let root_id = nfs.root_dir();

    // Create a symlink
    let (link_id, attrs) = nfs.symlink(root_id, b"link", b"/target/path").await?;

    // Verify it's a symlink (NF3LNK = 5)
    assert_eq!(attrs.ftype as u32, 5, "created item should be symlink");

    // Read the symlink
    let read_result = nfs.readlink(link_id).await?;
    assert_eq!(
        &read_result.data[..],
        b"/target/path",
        "readlink should return correct target"
    );

    Ok(())
}

#[tokio::test]
async fn test_nfs_setattr_operation() -> color_eyre::Result<()> {
    init_test();

    let ctx = TestOverlay::new()?;
    let nfs = NfsOverlay::new(ctx.overlay);
    let root_id = nfs.root_dir();

    // Create a file
    let (file_id, _) = nfs.create(root_id, b"attrs.txt").await?;

    // Write some data
    nfs.write(file_id, 0, b"hello world").await?;

    // Truncate using setattr
    let mut sattr = nfsserve::nfs::sattr3::default();
    sattr.size = nfsserve::nfs::set_size3::size(5);

    let attrs = nfs.setattr(file_id, sattr).await?;
    assert_eq!(attrs.size, 5, "file should be truncated to 5 bytes");

    // Verify content
    let read_result = nfs.read(file_id, 0, 100).await?;
    assert_eq!(
        &read_result.data[..],
        b"hello",
        "content should be truncated"
    );

    Ok(())
}
