mod helpers;

use helpers::{TestOverlay, init_test};
use loaf::db::ItemType;

#[test]
fn test_filename_with_spaces() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "file with spaces.txt", ItemType::File, 0o644)?;

    // Verify lookup works
    let lookup_id = ctx.overlay.lookup(root_id, "file with spaces.txt")?;
    assert_eq!(lookup_id, file_id, "lookup should find file with spaces");

    // Verify write and read work
    ctx.overlay.write(file_id, 0, b"data in spaced file")?;
    let mut buf = vec![0u8; 50];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"data in spaced file");

    Ok(())
}

#[test]
fn test_filename_with_unicode() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Test emoji
    let emoji_id = ctx
        .overlay
        .create(root_id, "emoji_🚀.txt", ItemType::File, 0o644)?;
    let lookup = ctx.overlay.lookup(root_id, "emoji_🚀.txt")?;
    assert_eq!(lookup, emoji_id, "emoji filename should work");

    // Test CJK characters
    let cjk_id = ctx
        .overlay
        .create(root_id, "文件.txt", ItemType::File, 0o644)?;
    let lookup = ctx.overlay.lookup(root_id, "文件.txt")?;
    assert_eq!(lookup, cjk_id, "CJK filename should work");

    // Test mixed unicode
    let mixed_id = ctx
        .overlay
        .create(root_id, "café_☕_文件.md", ItemType::File, 0o644)?;
    let lookup = ctx.overlay.lookup(root_id, "café_☕_文件.md")?;
    assert_eq!(lookup, mixed_id, "mixed unicode filename should work");

    Ok(())
}

#[test]
fn test_filename_with_special_chars() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Test common special characters
    let names = vec![
        "file-with-dashes.txt",
        "file_with_underscores.txt",
        "file.with.dots.txt",
        "file@symbol.txt",
        "file#hash.txt",
        "file$dollar.txt",
        "file%percent.txt",
        "file&ampersand.txt",
        "file(parens).txt",
        "file[brackets].txt",
        "file{braces}.txt",
    ];

    for name in names {
        let file_id = ctx.overlay.create(root_id, name, ItemType::File, 0o644)?;
        let lookup = ctx.overlay.lookup(root_id, name)?;
        assert_eq!(lookup, file_id, "special char filename {name} should work");
    }

    Ok(())
}

#[test]
fn test_very_long_filename() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create 255-character filename (typical filesystem limit)
    let long_name = "a".repeat(251) + ".txt"; // 255 total
    assert_eq!(long_name.len(), 255, "filename should be exactly 255 bytes");

    let file_id = ctx
        .overlay
        .create(root_id, &long_name, ItemType::File, 0o644)?;

    // Verify lookup works
    let lookup_id = ctx.overlay.lookup(root_id, &long_name)?;
    assert_eq!(lookup_id, file_id, "long filename lookup should work");

    // Verify operations work
    ctx.overlay.write(file_id, 0, b"long filename test")?;
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, 18);

    Ok(())
}

#[test]
fn test_large_file_write_and_read() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "large.bin", ItemType::File, 0o644)?;

    // Write 1MB of data
    let one_mb = 1024 * 1024;
    let data: Vec<u8> = (0..one_mb).map(|i| (i % 256) as u8).collect();

    let n = ctx.overlay.write(file_id, 0, &data)?;
    assert_eq!(n, one_mb, "should write full 1MB");

    // Verify size
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, one_mb as u64, "size should be 1MB");

    // Read back in chunks and verify
    let chunk_size = 64 * 1024; // 64KB chunks
    let mut buf = vec![0u8; chunk_size];

    for offset in (0..one_mb).step_by(chunk_size) {
        let n = ctx.overlay.read(file_id, offset as u64, &mut buf)?;
        let expected_size = std::cmp::min(chunk_size, one_mb - offset);
        assert_eq!(
            n, expected_size,
            "chunk at offset {offset} should have correct size"
        );

        // Verify content
        for (i, &byte) in buf[..n].iter().enumerate() {
            let expected = ((offset + i) % 256) as u8;
            assert_eq!(
                byte,
                expected,
                "byte at position {} should match pattern",
                offset + i
            );
        }
    }

    Ok(())
}

#[test]
fn test_many_files_in_directory() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create 200 files
    let num_files = 200;
    let mut file_ids = std::collections::HashMap::new();

    for i in 0..num_files {
        let name = format!("file_{:04}.txt", i);
        let file_id = ctx.overlay.create(root_id, &name, ItemType::File, 0o644)?;
        file_ids.insert(name.clone(), file_id);

        // Write unique content to each file
        let content = format!("content_{}", i);
        ctx.overlay.write(file_id, 0, content.as_bytes())?;
    }

    // Verify readdir shows all files
    let entries = ctx.overlay.readdir(root_id)?;
    assert!(
        entries.len() >= num_files,
        "readdir should list at least {} files, found {}",
        num_files,
        entries.len()
    );

    // Verify we can look up each file
    for i in 0..num_files {
        let name = format!("file_{:04}.txt", i);
        let lookup_id = ctx.overlay.lookup(root_id, &name)?;
        assert_eq!(lookup_id, file_ids[&name], "lookup should find file {name}");

        // Verify content
        let expected = format!("content_{}", i);
        let mut buf = vec![0u8; 100];
        let n = ctx.overlay.read(lookup_id, 0, &mut buf)?;
        assert_eq!(
            &buf[..n],
            expected.as_bytes(),
            "file {name} should have correct content"
        );
    }

    Ok(())
}

#[test]
fn test_deep_directory_nesting() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let mut current_id = ctx.root_id();

    // Create 15 levels of nested directories
    let depth = 15;
    let mut dir_ids = vec![current_id];

    for i in 0..depth {
        let name = format!("level{}", i);
        let dir_id = ctx
            .overlay
            .create(current_id, &name, ItemType::Directory, 0o755)?;
        dir_ids.push(dir_id);
        current_id = dir_id;
    }

    // Create a file in the deepest directory
    let file_id = ctx
        .overlay
        .create(current_id, "deep_file.txt", ItemType::File, 0o644)?;

    // Write and read from deeply nested file
    ctx.overlay.write(file_id, 0, b"deep nesting test")?;
    let mut buf = vec![0u8; 50];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"deep nesting test");

    // Verify we can traverse back up
    for i in (0..depth).rev() {
        let parent_id = dir_ids[i];
        let entries = ctx.overlay.readdir(parent_id)?;
        assert!(!entries.is_empty(), "level {} should have children", i);
    }

    Ok(())
}

#[test]
fn test_empty_file_operations() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create empty file
    let file_id = ctx
        .overlay
        .create(root_id, "empty.txt", ItemType::File, 0o644)?;

    // Verify size is 0
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, 0, "empty file should have size 0");

    // Read from empty file
    let mut buf = vec![0u8; 10];
    let n = ctx.overlay.read(file_id, 0, &mut buf)?;
    assert_eq!(n, 0, "reading empty file should return 0 bytes");

    // Write to empty file
    ctx.overlay.write(file_id, 0, b"now not empty")?;
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, 13);

    // Truncate back to empty
    ctx.overlay.setattr(file_id, None, Some(0), None, None)?;
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, 0, "truncated file should be empty");

    Ok(())
}

#[test]
fn test_directory_with_mixed_content() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    // Create directory with files, subdirectories, and symlinks
    ctx.overlay
        .create(root_id, "file1.txt", ItemType::File, 0o644)?;
    ctx.overlay
        .create(root_id, "file2.txt", ItemType::File, 0o644)?;
    ctx.overlay
        .create(root_id, "subdir", ItemType::Directory, 0o755)?;
    ctx.overlay.symlink(root_id, "link", "/target")?;

    // Verify readdir shows all types
    let entries = ctx.overlay.readdir(root_id)?;
    let names: std::collections::HashSet<_> = entries.iter().map(|e| e.name.as_str()).collect();

    assert!(names.contains("file1.txt"), "should list file1.txt");
    assert!(names.contains("file2.txt"), "should list file2.txt");
    assert!(names.contains("subdir"), "should list subdir");
    assert!(names.contains("link"), "should list link");

    // Verify types are correct
    for entry in &entries {
        let attrs = ctx.overlay.getattr(entry.inode_id)?;
        match entry.name.as_str() {
            "file1.txt" | "file2.txt" => {
                assert_eq!(
                    attrs.item_type,
                    ItemType::File,
                    "{} should be file",
                    entry.name
                );
            }
            "subdir" => {
                assert_eq!(
                    attrs.item_type,
                    ItemType::Directory,
                    "subdir should be directory"
                );
            }
            "link" => {
                assert_eq!(attrs.item_type, ItemType::Symlink, "link should be symlink");
            }
            _ => {}
        }
    }

    Ok(())
}

#[test]
fn test_zero_byte_write() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "zero.txt", ItemType::File, 0o644)?;

    // Write 0 bytes
    let n = ctx.overlay.write(file_id, 0, b"")?;
    assert_eq!(n, 0, "zero-byte write should return 0");

    // Verify file is still empty
    let attrs = ctx.overlay.getattr(file_id)?;
    assert_eq!(attrs.size, 0, "file should still be empty");

    Ok(())
}

#[test]
fn test_large_offset_write() -> color_eyre::Result<()> {
    init_test();

    let mut ctx = TestOverlay::new()?;
    let root_id = ctx.root_id();

    let file_id = ctx
        .overlay
        .create(root_id, "sparse.txt", ItemType::File, 0o644)?;

    // Write at large offset (creates sparse file)
    let offset = 1024 * 1024; // 1MB offset
    ctx.overlay.write(file_id, offset, b"data at end")?;

    // Verify size includes the gap
    let attrs = ctx.overlay.getattr(file_id)?;
    assert!(
        attrs.size >= offset + 11,
        "file size should include sparse region"
    );

    // Read from sparse region (should be zeros)
    let mut buf = vec![0u8; 10];
    let n = ctx.overlay.read(file_id, 100, &mut buf)?;
    assert_eq!(n, 10, "should read from sparse region");
    assert_eq!(&buf[..n], &[0u8; 10], "sparse region should be zeros");

    // Read actual data
    let mut buf = vec![0u8; 20];
    let n = ctx.overlay.read(file_id, offset, &mut buf)?;
    assert_eq!(&buf[..n], b"data at end", "should read data at offset");

    Ok(())
}
