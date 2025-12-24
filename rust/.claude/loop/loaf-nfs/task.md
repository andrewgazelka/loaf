# Project: loaf-nfs

## Goal

Implement a complete Loaf NFS overlay filesystem in Rust. This provides a working alternative to the FSKit-based approach (which is blocked by Apple bugs on macOS 26) by using the NFS protocol via the `nfsserve` crate.

The CLI will allow users to:
1. Mount an overlay filesystem on any directory via NFS
2. Run commands in an isolated overlay sandbox
3. View, accept, or reject changes made in the overlay
4. All writes go to SQLite, reads fall through to the real filesystem

## Technology Stack

- Language: Rust (edition 2024, stable toolchain)
- Async Runtime: Tokio
- NFS Server: nfsserve 0.10.2
- Database: rusqlite 0.38.0 (bundled SQLite)
- Error Handling: eyre/color-eyre
- Logging: tracing + tracing-subscriber
- CLI: clap 4 (derive)

## Architecture

The project has three main modules:

```
src/
  main.rs      - CLI entry point (clap), async runtime setup
  db.rs        - SQLite database layer (schema, queries)
  overlay.rs   - OverlayFs struct (path/inode mapping, overlay logic)
  nfs.rs       - NFSFileSystem trait impl, NFS server wrapper
```

### Data Flow

```
                    mount_nfs
User CLI -----> NFS Server (localhost:port) -----> NFSFileSystem trait
                                                         |
                                         +---------------+---------------+
                                         |                               |
                                   OverlayFs                        Real FS
                                   (SQLite)                       (passthrough)
```

### Key Abstractions

1. **Database** (`db.rs`) - SQLite operations for overlay state
   - Tables: inodes, file_data, whiteouts, overlay_config
   - Path-based queries for looking up/creating entries

2. **OverlayFs** (`overlay.rs`) - Inode-based overlay logic
   - Maps inodes <-> paths bidirectionally
   - Checks SQLite first, falls through to real FS
   - Handles whiteouts for deletions

3. **NfsOverlay** (`nfs.rs`) - NFSFileSystem implementation
   - Wraps OverlayFs with NFS-specific types
   - Converts between internal types and nfsserve types
   - Thread-safe via RwLock

## Patterns to Follow

### Error Handling
- Use `color_eyre::eyre::Result` for all fallible operations
- Add context with `.wrap_err()` / `.wrap_err_with()`
- Map to `nfsstat3` errors at the NFS boundary

### Async
- Use `async_trait` for NFSFileSystem (required by nfsserve)
- Use `tokio::sync::RwLock` for shared mutable state
- Server runs on tokio runtime

### Logging
- Use `tracing::info!`, `tracing::debug!`, `tracing::error!`
- Enable with `RUST_LOG=debug` environment variable

### NFS Specifics
- fileid3 (u64) = inode number
- Root directory must have fileid = 1 (by convention, cannot be 0)
- fattr3 contains all file attributes (mode, size, times, etc.)
- ReadDirResult pagination: `start_after` is the last returned fileid

## Key Decisions

1. **Blocking SQLite in async context**: Use `tokio::task::spawn_blocking` or `blocking` crate for database operations to avoid blocking the async runtime.

2. **Inode allocation**: Start at 1_000_000 for dynamically allocated inodes. Reserve 1 for root.

3. **Thread safety**: OverlayFs wrapped in `Arc<RwLock<OverlayFs>>` for concurrent NFS access.

4. **Port selection**: Default to 0 (OS-assigned), report actual port. Allow CLI override.

5. **Mount command**: Use `mount_nfs` on macOS with specific options for NFSv3 compatibility.

## External Dependencies

- macOS `mount_nfs` command for mounting
- macOS `umount` command for unmounting
- Localhost network (127.0.0.1)

## CLI Commands

```
loaf mount <path>           # Start NFS server, mount overlay on directory
loaf unmount <path>         # Unmount and stop NFS server
loaf run <cmd> [args...]    # Run command in temporary overlay, accept/reject
loaf diff [overlay.loaf]    # Show pending changes
loaf accept [overlay.loaf]  # Apply changes to real filesystem
loaf reject [overlay.loaf]  # Discard all changes
```

## Notes for Implementers

### NFSFileSystem Trait Methods

Required methods to implement:
- `capabilities()` -> `VFSCapabilities::ReadWrite`
- `root_dir()` -> `fileid3` (return 1)
- `lookup(dirid, filename)` -> find child inode
- `getattr(id)` -> return fattr3 with all metadata
- `setattr(id, sattr3)` -> update attributes
- `read(id, offset, count)` -> return (Vec<u8>, eof)
- `write(id, offset, data)` -> return fattr3
- `create(dirid, filename, attr)` -> create file
- `create_exclusive(dirid, filename)` -> atomic create
- `mkdir(dirid, dirname)` -> create directory
- `remove(dirid, filename)` -> delete file/dir
- `rename(from_dir, from_name, to_dir, to_name)` -> rename
- `readdir(dirid, start_after, max_entries)` -> paginated listing
- `symlink(dirid, linkname, target, attr)` -> create symlink
- `readlink(id)` -> read symlink target

### Mount Command (macOS)

```bash
mount_nfs -o nolocks,vers=3,tcp,rsize=131072,port=PORT,mountport=PORT \
    localhost:/ /mount/point
```

### Common Pitfalls

1. **fattr3.fileid must match the fileid3 used in lookup/readdir** - consistency is critical
2. **mode field is u32** - includes file type bits (S_IFREG, S_IFDIR, etc.)
3. **nfstime3 has seconds (u32) and nseconds (u32)** - not i64!
4. **readdir pagination**: Return entries AFTER `start_after`, not including it
5. **filename3 is Vec<u8>** - convert to/from String carefully (UTF-8)

### Testing

- Manual testing: mount, create files, verify in SQLite
- Use `sqlite3` CLI to inspect overlay database
- Check `RUST_LOG=debug` output for NFS operations
