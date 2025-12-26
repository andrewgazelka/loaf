mod helpers;

#[test]
fn test_git_init_creates_structure() -> color_eyre::Result<()> {
    helpers::init_test();
    let mut ctx = helpers::TestOverlay::new()?;

    // Run git init via NFS mount would be ideal, but for now test via overlay directly
    // Create .git directory structure via overlay
    let root_id = ctx.root_id();
    let git_id = ctx.overlay.mkdir(root_id, ".git", 0o755)?;

    // Verify .git directory exists
    let attrs = ctx.overlay.getattr(git_id)?;
    assert_eq!(attrs.item_type, loaf::db::ItemType::Directory);

    // Verify base directory is unchanged
    let base_git = ctx.base_path.join(".git");
    assert!(
        !base_git.exists(),
        ".git should only exist in overlay, not base"
    );

    Ok(())
}

#[test]
fn test_git_workflow_via_overlay() -> color_eyre::Result<()> {
    helpers::init_test();
    let mut ctx = helpers::TestOverlay::new()?;

    // We'll simulate a git workflow by creating the files git would create
    // This tests that the overlay correctly handles git-like operations

    let root_id = ctx.root_id();

    // Create a file to "commit"
    let file_id = ctx
        .overlay
        .create(root_id, "test.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"Hello, world!")?;

    // Create .git structure
    let git_id = ctx.overlay.mkdir(root_id, ".git", 0o755)?;
    let _objects_id = ctx.overlay.mkdir(git_id, "objects", 0o755)?;
    let _refs_id = ctx.overlay.mkdir(git_id, "refs", 0o755)?;

    // Create HEAD file
    let head_id = ctx
        .overlay
        .create(git_id, "HEAD", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(head_id, 0, b"ref: refs/heads/main\n")?;

    // Verify structure
    let git_entries = ctx.overlay.readdir(git_id)?;
    let git_names: std::collections::HashSet<_> =
        git_entries.iter().map(|e| e.name.as_str()).collect();
    assert!(git_names.contains("objects"));
    assert!(git_names.contains("refs"));
    assert!(git_names.contains("HEAD"));

    // Verify HEAD content
    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(head_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"ref: refs/heads/main\n");

    // Verify base is unchanged
    assert!(!ctx.base_path.join(".git").exists());
    assert!(!ctx.base_path.join("test.txt").exists());

    Ok(())
}

#[test]
fn test_git_add_staging() -> color_eyre::Result<()> {
    helpers::init_test();
    let mut ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create file to stage
    let file_id = ctx
        .overlay
        .create(root_id, "new_file.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay.write(file_id, 0, b"staged content")?;

    // Simulate git index by creating .git/index
    let git_id = ctx.overlay.mkdir(root_id, ".git", 0o755)?;
    let index_id = ctx
        .overlay
        .create(git_id, "index", loaf::db::ItemType::File, 0o644)?;

    // Write mock index data
    ctx.overlay.write(index_id, 0, b"DIRC\x00\x00\x00\x02")?; // Mock git index header

    // Verify index was written to overlay
    let mut buf = vec![0u8; 8];
    let n = ctx.overlay.read(index_id, 0, &mut buf)?;
    assert_eq!(n, 8);
    assert_eq!(&buf[..4], b"DIRC");

    Ok(())
}

#[test]
fn test_git_commit_creates_objects() -> color_eyre::Result<()> {
    helpers::init_test();
    let mut ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create .git/objects structure
    let git_id = ctx.overlay.mkdir(root_id, ".git", 0o755)?;
    let objects_id = ctx.overlay.mkdir(git_id, "objects", 0o755)?;

    // Simulate creating a blob object (first 2 chars of SHA are directory)
    let obj_dir_id = ctx.overlay.mkdir(objects_id, "ab", 0o755)?;
    let blob_id = ctx.overlay.create(
        obj_dir_id,
        "cdef1234567890",
        loaf::db::ItemType::File,
        0o444,
    )?;

    // Write compressed blob data (simplified)
    ctx.overlay.write(blob_id, 0, b"blob 13\x00Hello, world!")?;

    // Verify object file exists
    let obj_dir_entries = ctx.overlay.readdir(obj_dir_id)?;
    assert_eq!(obj_dir_entries.len(), 1);
    assert_eq!(obj_dir_entries[0].name, "cdef1234567890");

    // Verify base is unchanged
    assert!(!ctx.base_path.join(".git/objects/ab").exists());

    Ok(())
}

#[test]
fn test_git_status_via_readdir() -> color_eyre::Result<()> {
    helpers::init_test();
    let mut ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create some files in overlay (untracked/modified)
    ctx.overlay
        .create(root_id, "untracked.txt", loaf::db::ItemType::File, 0o644)?;
    ctx.overlay
        .create(root_id, "modified.txt", loaf::db::ItemType::File, 0o644)?;

    // Create .git
    ctx.overlay.mkdir(root_id, ".git", 0o755)?;

    // Git status would readdir to find untracked files
    let entries = ctx.overlay.readdir(root_id)?;

    let names: std::collections::HashSet<_> = entries
        .iter()
        .map(|e| e.name.as_str())
        .filter(|n| *n != ".git")
        .collect();

    assert!(names.contains("untracked.txt"));
    assert!(names.contains("modified.txt"));

    Ok(())
}

#[test]
fn test_git_diff_via_read() -> color_eyre::Result<()> {
    helpers::init_test();
    let mut ctx = helpers::TestOverlay::new()?;

    // Create a file in base (committed version)
    ctx.create_base_file("committed.txt", b"original content")?;

    let root_id = ctx.root_id();

    // Modify via overlay (working tree version)
    let file_id = ctx.overlay.lookup(root_id, "committed.txt")?;
    ctx.overlay.write(file_id, 0, b"modified content")?;

    // Read from overlay (git diff would compare these)
    let mut overlay_buf = vec![0u8; 100];
    let n = ctx.overlay.read(file_id, 0, &mut overlay_buf)?;
    assert_eq!(&overlay_buf[..n], b"modified content");

    // Read from base (original)
    let base_content = std::fs::read(ctx.base_path.join("committed.txt"))?;
    assert_eq!(&base_content, b"original content");

    // This demonstrates copy-on-write for diffs

    Ok(())
}

#[test]
fn test_git_log_via_refs() -> color_eyre::Result<()> {
    helpers::init_test();
    let mut ctx = helpers::TestOverlay::new()?;

    let root_id = ctx.root_id();

    // Create .git/refs/heads structure
    let git_id = ctx.overlay.mkdir(root_id, ".git", 0o755)?;
    let refs_id = ctx.overlay.mkdir(git_id, "refs", 0o755)?;
    let heads_id = ctx.overlay.mkdir(refs_id, "heads", 0o755)?;

    // Create main branch ref
    let main_id = ctx
        .overlay
        .create(heads_id, "main", loaf::db::ItemType::File, 0o644)?;
    let commit_sha = b"abcdef1234567890abcdef1234567890abcdef12\n";
    ctx.overlay.write(main_id, 0, commit_sha)?;

    // Read commit SHA (git log would use this)
    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(main_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], commit_sha);

    // Verify structure
    let heads_entries = ctx.overlay.readdir(heads_id)?;
    assert_eq!(heads_entries.len(), 1);
    assert_eq!(heads_entries[0].name, "main");

    Ok(())
}

#[test]
fn test_git_preserves_base_state() -> color_eyre::Result<()> {
    helpers::init_test();
    let mut ctx = helpers::TestOverlay::new()?;

    // Create initial state in base
    ctx.create_base_file("README.md", b"# Original Repo\n")?;
    ctx.create_base_dir(".git")?;
    std::fs::write(
        ctx.base_path.join(".git/config"),
        b"[core]\n\tbare = false\n",
    )?;

    let root_id = ctx.root_id();

    // Make changes via overlay
    let readme_id = ctx.overlay.lookup(root_id, "README.md")?;
    let new_readme = b"# Modified Repo\n";
    ctx.overlay.write(readme_id, 0, new_readme)?;
    // Truncate to new size since partial writes preserve trailing bytes
    ctx.overlay
        .setattr(readme_id, None, Some(new_readme.len() as u64), None, None)?;

    let git_id = ctx.overlay.lookup(root_id, ".git")?;
    let config_id = ctx.overlay.lookup(git_id, "config")?;
    let new_config = b"[core]\n\tbare = true\n";
    ctx.overlay.write(config_id, 0, new_config)?;
    // Truncate to new size since partial writes preserve trailing bytes
    ctx.overlay
        .setattr(config_id, None, Some(new_config.len() as u64), None, None)?;

    // Verify overlay has changes
    let mut buf = vec![0u8; 100];
    let n = ctx.overlay.read(readme_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"# Modified Repo\n");

    let n = ctx.overlay.read(config_id, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"[core]\n\tbare = true\n");

    // Verify base is UNCHANGED
    let base_readme = std::fs::read(ctx.base_path.join("README.md"))?;
    assert_eq!(&base_readme, b"# Original Repo\n");

    let base_config = std::fs::read(ctx.base_path.join(".git/config"))?;
    assert_eq!(&base_config, b"[core]\n\tbare = false\n");

    Ok(())
}
