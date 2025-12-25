# Project: loaf-tests

## Goal

Create a comprehensive, production-ready integration test suite for the loaf overlay filesystem. The loaf project is a Rust implementation of an overlay filesystem using SQLite for copy-on-write semantics, exposed via NFS. Tests should exercise the full stack including:

1. Normal git repository operations through the overlay
2. All filesystem operations (create, read, write, delete, rename, truncate, append)
3. Directory operations (mkdir, rmdir, nested directories, listing)
4. Symlink handling (create, read, delete, relative/absolute paths)
5. Overlay semantics (whiteouts, copy-on-write, passthrough reads)
6. Edge cases (special characters, large files, many files, concurrent access)
7. Error handling (permission errors, invalid paths, non-existent files)

## Technology Stack

- Language: Rust 2024 edition
- Test Framework: Built-in `#[test]` and `#[tokio::test]` with cargo nextest
- Error Handling: `color_eyre` (NOT `anyhow`)
- Async Runtime: tokio with multi-threaded runtime
- Temp Files: `tempfile` crate (already a dependency)
- Process Execution: `tokio::process::Command`

## Architecture

The codebase structure:
- `src/lib.rs` - Re-exports db, nfs, overlay modules
- `src/db.rs` - SQLite database layer (schema, queries, inodes, whiteouts)
- `src/overlay.rs` - High-level overlay filesystem API (inode + path based)
- `src/nfs.rs` - NFS protocol implementation wrapping overlay
- `src/main.rs` - CLI with mount, run, diff, accept, reject commands
- `tests/integration.rs` - Existing integration tests (basic coverage)

## Key Components

### Database Layer (`db.rs`)
- `Database` struct wraps rusqlite connection
- SQLite tables: `inodes`, `file_data`, `xattrs`, `whiteouts`, `overlay_config`
- Path-based operations: `create_by_path`, `read_by_path`, `write_by_path`, `rename_by_path`
- Whiteout management: `add_whiteout`, `remove_whiteout`, `is_whiteout`

### Overlay Layer (`overlay.rs`)
- `OverlayFs` struct with inode-to-path mapping
- Combines SQLite overlay with real filesystem passthrough
- Key methods: `create`, `read`, `write`, `remove`, `rename`, `symlink`, `readdir`, `setattr`
- Copy-on-write: reads pass through to real FS, writes copy to SQLite first

### NFS Layer (`nfs.rs`)
- `NfsOverlay` wraps OverlayFs with thread-safe Arc<Mutex<>>
- Implements `NFSFileSystem` trait from `nfsserve` crate
- `NfsServer::start()` binds to port and returns task handle
- `mount_nfs()` / `unmount_nfs()` shell out to mount_nfs/umount

## Patterns to Follow

### Error Handling
- Use `color_eyre::Result<()>` for all test functions
- Use `.wrap_err()` / `.wrap_err_with()` for context on every `?`
- Never use bare `.unwrap()` - use `.expect("invariant reason")` if truly impossible

### Test Structure
- Each test creates its own `tempfile::tempdir()`
- Create `base` directory within temp dir for overlay base path
- Create `.loaf` database file within temp dir
- Clean up is automatic via `tempfile::TempDir` drop

### Assertions
- Use `assert!`, `assert_eq!` with descriptive messages
- For file content comparisons, truncate buffers to actual read size
- Check both success cases and error cases

### Logging in Tests
- Call `init_test()` helper at start of each test (installs color_eyre once)
- Use `println!` for test progress markers
- Tracing is configured but not needed in tests

## Test Categories

### 1. Git Repository Operations
Test that git commands work correctly through overlay:
- `git init` creates proper .git structure
- `git add`, `git commit` work on overlay files
- `git status`, `git diff` show correct state
- Changes in overlay don't affect real git state

### 2. File Operations
- Create file, verify exists
- Write data, read back correctly
- Append data (write at offset)
- Truncate file (via setattr)
- Overwrite existing file
- Delete file (creates whiteout)

### 3. Directory Operations
- Create directory
- Create nested directories
- List directory contents
- Remove empty directory
- Remove non-empty directory (error expected)

### 4. Symlink Operations
- Create symlink with absolute target
- Create symlink with relative target
- Read symlink target
- Delete symlink
- Symlink to non-existent target (valid)

### 5. Overlay Semantics
- Read passthrough: read from real FS when not in overlay
- Write copies: write to overlay, real FS unchanged
- Whiteout: delete real file, verify hidden
- Whiteout removal: create file at whiteout path, whiteout cleared

### 6. Edge Cases
- Filenames with spaces
- Filenames with unicode characters
- Filenames with special chars (except / and NUL)
- Large file (1MB+) write and read
- Many files (100+) in single directory
- Deep directory nesting (10+ levels)

### 7. Error Handling
- Read non-existent file (error)
- Write to read-only permission file (may error)
- Remove non-existent file (error)
- Lookup in non-existent directory (error)
- Create file in non-existent parent (error)

## External Dependencies

- No external services needed
- Tests are self-contained with tempfile
- No testcontainers required (SQLite is embedded)

## Notes for Implementers

### Existing Tests
There are already basic tests in `tests/integration.rs` and unit tests in `db.rs` and `overlay.rs`. The new tests should complement these, not duplicate them. Focus on:
- Git operations (completely missing)
- Edge cases (mostly missing)
- Error handling (minimal coverage)
- Concurrent access (not tested)

### Git Testing Notes
- Use `std::process::Command` or `tokio::process::Command` for git
- Set `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`, `GIT_COMMITTER_NAME`, `GIT_COMMITTER_EMAIL` env vars
- Use `git init --initial-branch=main` for consistent behavior

### NFS Mount Testing
The NFS mount requires `mount_nfs` command which needs elevated privileges on some systems. Tests that actually mount should:
- Check if mount_nfs is available
- Skip gracefully if no permission
- Always clean up (unmount) even on test failure

### Parallel Safety
Tests run in parallel by default with cargo nextest. Each test must:
- Use unique tempdir
- Not use fixed ports (use port 0 for OS-assigned)
- Not depend on global state
