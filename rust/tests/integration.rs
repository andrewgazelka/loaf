use color_eyre::eyre::WrapErr as _;

fn init_test() {
    // Install color_eyre only once - ignore error if already installed
    let _ = color_eyre::install();
}

#[tokio::test]
async fn test_nfs_server_basic_workflow() -> color_eyre::Result<()> {
    init_test();

    // Create temporary base directory
    let temp_dir = tempfile::tempdir()
        .wrap_err("failed to create temp directory")?;
    let base_path = temp_dir.path().join("base");
    std::fs::create_dir(&base_path)
        .wrap_err("failed to create base directory")?;

    // Create a file in the base directory to verify passthrough reads
    std::fs::write(base_path.join("existing.txt"), b"original content")
        .wrap_err("failed to create existing file")?;

    // Create overlay database
    let overlay_path = temp_dir.path().join("test.loaf");
    let overlay = loaf::overlay::OverlayFs::new(&overlay_path, &base_path)
        .wrap_err("failed to create overlay")?;

    // Start NFS server on random port
    let (server, server_task) = loaf::nfs::NfsServer::start(overlay, None).await
        .wrap_err("failed to start NFS server")?;

    println!("✓ NFS server started on port {}", server.port);

    // Stop server
    server_task.abort();
    let _ = server_task.await; // Ignore abort error

    println!("✓ NFS server stopped successfully");

    Ok(())
}

#[tokio::test]
async fn test_overlay_operations() -> color_eyre::Result<()> {
    init_test();

    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path().join("base");
    std::fs::create_dir(&base_path)?;

    // Create existing file in base
    std::fs::write(base_path.join("existing.txt"), b"base content")?;

    let overlay_path = temp_dir.path().join("test.loaf");
    let mut overlay = loaf::overlay::OverlayFs::new(&overlay_path, &base_path)?;

    // Test 1: Lookup existing file (passthrough)
    let root_id = overlay.root_id();
    let file_id = overlay.lookup(root_id, "existing.txt")
        .wrap_err("failed to lookup existing file")?;
    println!("✓ Looked up existing file: inode {}", file_id);

    // Test 2: Read existing file content
    let mut buf = vec![0u8; 100];
    let n = overlay.read(file_id, 0, &mut buf)
        .wrap_err("failed to read file")?;
    buf.truncate(n);
    assert_eq!(&buf, b"base content");
    println!("✓ Read existing file via passthrough");

    // Test 3: Create new file
    let new_file_id = overlay.create(root_id, "new.txt", loaf::db::ItemType::File, 0o644)
        .wrap_err("failed to create file")?;
    println!("✓ Created new file: inode {}", new_file_id);

    // Test 4: Write to new file
    overlay.write(new_file_id, 0, b"hello world")
        .wrap_err("failed to write to file")?;
    println!("✓ Wrote data to new file");

    // Test 5: Read back
    let mut buf = vec![0u8; 20];
    let n = overlay.read(new_file_id, 0, &mut buf)?;
    buf.truncate(n);
    assert_eq!(&buf, b"hello world");
    println!("✓ Read back written data");

    // Test 6: Create directory
    let dir_id = overlay.mkdir(root_id, "testdir", 0o755)
        .wrap_err("failed to create directory")?;
    println!("✓ Created directory: inode {}", dir_id);

    // Test 7: List directory
    let entries = overlay.readdir(root_id)
        .wrap_err("failed to list directory")?;
    // The overlay should contain new.txt and testdir, but existing.txt is only in base
    // so readdir might only show overlay entries
    assert!(!entries.is_empty(), "should have at least some entries");
    println!("✓ Listed directory: {} entries", entries.len());

    // Test 8: Delete file (creates whiteout)
    overlay.remove(new_file_id)
        .wrap_err("failed to remove file")?;
    println!("✓ Removed file (created whiteout)");

    // Test 9: Verify file is gone
    let lookup_result = overlay.lookup(root_id, "new.txt");
    assert!(lookup_result.is_err(), "deleted file should not be found");
    println!("✓ Deleted file is no longer accessible");

    // Test 10: Create symlink
    let link_id = overlay.symlink(root_id, "link.txt", "/target/path")
        .wrap_err("failed to create symlink")?;
    let target = overlay.readlink(link_id)
        .wrap_err("failed to read symlink")?;
    assert_eq!(target, "/target/path");
    println!("✓ Created and read symlink");

    Ok(())
}

#[tokio::test]
async fn test_overlay_rename() -> color_eyre::Result<()> {
    init_test();

    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path().join("base");
    std::fs::create_dir(&base_path)?;

    let overlay_path = temp_dir.path().join("test.loaf");
    let mut overlay = loaf::overlay::OverlayFs::new(&overlay_path, &base_path)?;

    let root_id = overlay.root_id();

    // Create file
    let file_id = overlay.create(root_id, "old.txt", loaf::db::ItemType::File, 0o644)?;
    overlay.write(file_id, 0, b"rename test")?;

    // Rename file
    overlay.rename(root_id, "old.txt", root_id, "new.txt")
        .wrap_err("failed to rename file")?;

    // Verify old name doesn't exist
    assert!(overlay.lookup(root_id, "old.txt").is_err(), "old file still exists");

    // Verify new name exists
    let new_id = overlay.lookup(root_id, "new.txt")
        .wrap_err("failed to lookup renamed file")?;

    // Verify content is preserved
    let mut buf = vec![0u8; 20];
    let n = overlay.read(new_id, 0, &mut buf)?;
    buf.truncate(n);
    assert_eq!(&buf, b"rename test");

    println!("✓ File rename works");

    Ok(())
}
