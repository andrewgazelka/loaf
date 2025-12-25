use color_eyre::eyre::WrapErr as _;

mod helpers;

#[test]
fn test_read_passthrough() -> color_eyre::Result<()> {
    helpers::init_test();

    let mut overlay = helpers::TestOverlay::new()?;

    // Create a file in the base filesystem
    overlay
        .create_base_file("existing.txt", b"base content")
        .wrap_err("failed to create base file")?;

    // Read it through the overlay without modifying
    let root_id = overlay.root_id();
    let file_id = overlay
        .overlay
        .lookup(root_id, "existing.txt")
        .wrap_err("failed to lookup base file through overlay")?;

    let mut buf = vec![0u8; 100];
    let n = overlay
        .overlay
        .read(file_id, 0, &mut buf)
        .wrap_err("failed to read base file through overlay")?;

    assert_eq!(n, 12, "read wrong number of bytes");
    assert_eq!(&buf[..n], b"base content", "content mismatch");

    // Verify file is NOT copied to overlay (passthrough read)
    let inodes = overlay
        .overlay
        .get_all_inodes()
        .wrap_err("failed to get all inodes")?;

    assert!(
        inodes.is_empty(),
        "passthrough read should not create overlay inode"
    );

    Ok(())
}

#[test]
fn test_write_triggers_copy() -> color_eyre::Result<()> {
    helpers::init_test();

    let mut overlay = helpers::TestOverlay::new()?;

    // Create a file in the base filesystem
    overlay
        .create_base_file("modify.txt", b"original content")
        .wrap_err("failed to create base file")?;

    // Modify it through overlay
    let root_id = overlay.root_id();
    let file_id = overlay
        .overlay
        .lookup(root_id, "modify.txt")
        .wrap_err("failed to lookup file")?;

    overlay
        .overlay
        .write(file_id, 0, b"modified")
        .wrap_err("failed to write to file")?;

    // Read back through overlay - should see modified version
    let mut buf = vec![0u8; 100];
    let n = overlay
        .overlay
        .read(file_id, 0, &mut buf)
        .wrap_err("failed to read modified file")?;

    assert_eq!(
        &buf[..n],
        b"modified content",
        "overlay should show modified content"
    );

    // Verify base file is unchanged
    let base_content =
        std::fs::read(overlay.base_path.join("modify.txt")).wrap_err("failed to read base file")?;
    assert_eq!(
        &base_content, b"original content",
        "base file should be unchanged"
    );

    // Verify file WAS copied to overlay
    let inodes = overlay
        .overlay
        .get_all_inodes()
        .wrap_err("failed to get all inodes")?;

    assert_eq!(inodes.len(), 1, "write should create overlay inode");
    assert_eq!(inodes[0].0, "/modify.txt", "wrong inode path");

    Ok(())
}

#[test]
fn test_whiteout_hides_base_file() -> color_eyre::Result<()> {
    helpers::init_test();

    let mut overlay = helpers::TestOverlay::new()?;

    // Create a file in the base filesystem
    overlay
        .create_base_file("delete_me.txt", b"will be hidden")
        .wrap_err("failed to create base file")?;

    // Verify it's visible through overlay
    let root_id = overlay.root_id();
    let file_id = overlay
        .overlay
        .lookup(root_id, "delete_me.txt")
        .wrap_err("file should be visible before deletion")?;

    // Remove it through overlay
    overlay
        .overlay
        .remove(file_id)
        .wrap_err("failed to remove file")?;

    // Verify it's now hidden
    let lookup_result = overlay.overlay.lookup(root_id, "delete_me.txt");
    assert!(
        lookup_result.is_err(),
        "file should be hidden after removal"
    );

    // Verify base file still exists
    let base_path = overlay.base_path.join("delete_me.txt");
    assert!(base_path.exists(), "base file should still exist");

    // Verify whiteout was created
    let whiteouts = overlay
        .overlay
        .get_all_whiteouts()
        .wrap_err("failed to get whiteouts")?;

    assert_eq!(whiteouts.len(), 1, "should have one whiteout");
    assert_eq!(whiteouts[0], "/delete_me.txt", "wrong whiteout path");

    Ok(())
}

#[test]
fn test_whiteout_cleared_on_create() -> color_eyre::Result<()> {
    helpers::init_test();

    let mut overlay = helpers::TestOverlay::new()?;

    // Create a file in base, then delete it through overlay
    overlay
        .create_base_file("recreate.txt", b"first version")
        .wrap_err("failed to create base file")?;

    let root_id = overlay.root_id();
    let file_id = overlay
        .overlay
        .lookup(root_id, "recreate.txt")
        .wrap_err("failed to lookup file")?;

    overlay
        .overlay
        .remove(file_id)
        .wrap_err("failed to remove file")?;

    // Verify whiteout exists
    let whiteouts = overlay
        .overlay
        .get_all_whiteouts()
        .wrap_err("failed to get whiteouts")?;
    assert_eq!(whiteouts.len(), 1, "whiteout should exist");

    // Create a new file at the same path
    let new_file_id = overlay
        .overlay
        .create(root_id, "recreate.txt", loaf::db::ItemType::File, 0o644)
        .wrap_err("failed to create new file")?;

    // Write to it
    overlay
        .overlay
        .write(new_file_id, 0, b"second version")
        .wrap_err("failed to write to new file")?;

    // Verify whiteout was cleared
    let whiteouts = overlay
        .overlay
        .get_all_whiteouts()
        .wrap_err("failed to get whiteouts after recreate")?;
    assert!(whiteouts.is_empty(), "whiteout should be cleared on create");

    // Verify file is now visible and has new content
    let file_id = overlay
        .overlay
        .lookup(root_id, "recreate.txt")
        .wrap_err("file should be visible after recreate")?;

    let mut buf = vec![0u8; 100];
    let n = overlay
        .overlay
        .read(file_id, 0, &mut buf)
        .wrap_err("failed to read recreated file")?;

    assert_eq!(&buf[..n], b"second version", "should have new content");

    Ok(())
}

#[test]
fn test_base_file_unchanged_after_overlay_write() -> color_eyre::Result<()> {
    helpers::init_test();

    let mut overlay = helpers::TestOverlay::new()?;

    // Create a file in base
    overlay
        .create_base_file("immutable.txt", b"base version")
        .wrap_err("failed to create base file")?;

    // Modify through overlay
    let root_id = overlay.root_id();
    let file_id = overlay
        .overlay
        .lookup(root_id, "immutable.txt")
        .wrap_err("failed to lookup file")?;

    overlay
        .overlay
        .write(file_id, 0, b"overlay version")
        .wrap_err("failed to write to overlay")?;

    // Verify base file is still unchanged
    let base_content = std::fs::read(overlay.base_path.join("immutable.txt"))
        .wrap_err("failed to read base file")?;
    assert_eq!(
        &base_content, b"base version",
        "base file must remain unchanged"
    );

    // Verify overlay has modified version
    let mut buf = vec![0u8; 100];
    let n = overlay
        .overlay
        .read(file_id, 0, &mut buf)
        .wrap_err("failed to read from overlay")?;
    assert_eq!(
        &buf[..n],
        b"overlay version",
        "overlay should have modified content"
    );

    Ok(())
}

#[test]
fn test_overlay_changes_visible_base_unchanged() -> color_eyre::Result<()> {
    helpers::init_test();

    let mut overlay = helpers::TestOverlay::new()?;

    // Create files in base
    overlay
        .create_base_file("base1.txt", b"base content 1")
        .wrap_err("failed to create base1")?;
    overlay
        .create_base_file("base2.txt", b"base content 2")
        .wrap_err("failed to create base2")?;

    // Create new file in overlay (not in base)
    let root_id = overlay.root_id();
    let new_file_id = overlay
        .overlay
        .create(root_id, "overlay_only.txt", loaf::db::ItemType::File, 0o644)
        .wrap_err("failed to create overlay-only file")?;

    overlay
        .overlay
        .write(new_file_id, 0, b"overlay content")
        .wrap_err("failed to write to overlay file")?;

    // Modify base1 through overlay
    let file1_id = overlay
        .overlay
        .lookup(root_id, "base1.txt")
        .wrap_err("failed to lookup base1")?;
    overlay
        .overlay
        .write(file1_id, 0, b"modified content 1")
        .wrap_err("failed to modify base1")?;

    // Verify:
    // 1. overlay_only.txt exists in overlay, not in base
    let overlay_only_path = overlay.base_path.join("overlay_only.txt");
    assert!(
        !overlay_only_path.exists(),
        "overlay-only file should not exist in base"
    );

    let oid = overlay
        .overlay
        .lookup(root_id, "overlay_only.txt")
        .wrap_err("overlay-only file should be visible")?;
    let mut buf = vec![0u8; 100];
    let n = overlay
        .overlay
        .read(oid, 0, &mut buf)
        .wrap_err("failed to read overlay-only file")?;
    assert_eq!(&buf[..n], b"overlay content", "wrong overlay-only content");

    // 2. base1 is modified in overlay, unchanged in base
    let base1_content = std::fs::read(overlay.base_path.join("base1.txt"))
        .wrap_err("failed to read base1 from base")?;
    assert_eq!(
        &base1_content, b"base content 1",
        "base1 in base should be unchanged"
    );

    let mut buf = vec![0u8; 100];
    let n = overlay
        .overlay
        .read(file1_id, 0, &mut buf)
        .wrap_err("failed to read base1 from overlay")?;
    assert_eq!(
        &buf[..n],
        b"modified content 1",
        "base1 in overlay should be modified"
    );

    // 3. base2 is unchanged (passthrough)
    let file2_id = overlay
        .overlay
        .lookup(root_id, "base2.txt")
        .wrap_err("failed to lookup base2")?;
    let mut buf = vec![0u8; 100];
    let n = overlay
        .overlay
        .read(file2_id, 0, &mut buf)
        .wrap_err("failed to read base2")?;
    assert_eq!(&buf[..n], b"base content 2", "base2 should be unchanged");

    Ok(())
}

#[test]
fn test_readdir_merges_overlay_and_base() -> color_eyre::Result<()> {
    helpers::init_test();

    let mut overlay = helpers::TestOverlay::new()?;

    // Create files in base
    overlay
        .create_base_file("base1.txt", b"base")
        .wrap_err("failed to create base1")?;
    overlay
        .create_base_file("base2.txt", b"base")
        .wrap_err("failed to create base2")?;

    // Create files in overlay
    let root_id = overlay.root_id();
    let overlay_file_id = overlay
        .overlay
        .create(root_id, "overlay1.txt", loaf::db::ItemType::File, 0o644)
        .wrap_err("failed to create overlay1")?;
    overlay
        .overlay
        .write(overlay_file_id, 0, b"overlay")
        .wrap_err("failed to write overlay1")?;

    let _overlay_dir_id = overlay
        .overlay
        .mkdir(root_id, "overlay_dir", 0o755)
        .wrap_err("failed to create overlay dir")?;

    // List directory
    let entries = overlay
        .overlay
        .readdir(root_id)
        .wrap_err("failed to readdir")?;

    let names: std::collections::HashSet<_> = entries.iter().map(|e| e.name.as_str()).collect();

    // Should see both base and overlay entries
    assert!(names.contains("base1.txt"), "should see base1.txt");
    assert!(names.contains("base2.txt"), "should see base2.txt");
    assert!(names.contains("overlay1.txt"), "should see overlay1.txt");
    assert!(names.contains("overlay_dir"), "should see overlay_dir");
    assert_eq!(entries.len(), 4, "should see exactly 4 entries");

    Ok(())
}

#[test]
fn test_readdir_excludes_whiteout_entries() -> color_eyre::Result<()> {
    helpers::init_test();

    let mut overlay = helpers::TestOverlay::new()?;

    // Create files in base
    overlay
        .create_base_file("keep.txt", b"keep")
        .wrap_err("failed to create keep")?;
    overlay
        .create_base_file("delete.txt", b"delete")
        .wrap_err("failed to create delete")?;
    overlay
        .create_base_file("also_delete.txt", b"delete")
        .wrap_err("failed to create also_delete")?;

    // Delete some files through overlay
    let root_id = overlay.root_id();
    let delete_id = overlay
        .overlay
        .lookup(root_id, "delete.txt")
        .wrap_err("failed to lookup delete")?;
    overlay
        .overlay
        .remove(delete_id)
        .wrap_err("failed to remove delete")?;

    let also_delete_id = overlay
        .overlay
        .lookup(root_id, "also_delete.txt")
        .wrap_err("failed to lookup also_delete")?;
    overlay
        .overlay
        .remove(also_delete_id)
        .wrap_err("failed to remove also_delete")?;

    // Create a new file in overlay
    let new_id = overlay
        .overlay
        .create(root_id, "new.txt", loaf::db::ItemType::File, 0o644)
        .wrap_err("failed to create new")?;
    overlay
        .overlay
        .write(new_id, 0, b"new")
        .wrap_err("failed to write new")?;

    // List directory
    let entries = overlay
        .overlay
        .readdir(root_id)
        .wrap_err("failed to readdir")?;

    let names: std::collections::HashSet<_> = entries.iter().map(|e| e.name.as_str()).collect();

    // Should see keep.txt and new.txt, but NOT delete.txt or also_delete.txt
    assert!(names.contains("keep.txt"), "should see keep.txt");
    assert!(names.contains("new.txt"), "should see new.txt");
    assert!(!names.contains("delete.txt"), "should NOT see deleted file");
    assert!(
        !names.contains("also_delete.txt"),
        "should NOT see deleted file"
    );
    assert_eq!(entries.len(), 2, "should see exactly 2 entries");

    Ok(())
}
