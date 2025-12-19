# loaf

Issues: @ISSUES.md
Follow: @mvp.md for mvp status and keep logging there

We have example implementation in @docs/FSKitSample

Contributing: @CONTRIBUTING.md

We are on macOS 26

## Constraints

- **Disabling SIP is off the table** - Solution must work with System Integrity Protection enabled

Fork your filesystem, let AI run wild, accept or reject changes. macOS equivalent of [poof](https://github.com/jarred-sumner/poof).

## Architecture — FSKit Overlay

Uses FSKit to implement a true overlay filesystem. NO APFS clones.

```
┌─────────────────────────────────────┐
│   App sees normal POSIX filesystem  │
├─────────────────────────────────────┤
│   FSKit (Apple's Swift framework)   │
├─────────────────────────────────────┤
│   Swift extension (LoafExtension)   │  ← Calls Zig via C FFI
├─────────────────────────────────────┤
│   C FFI bridge (loaf.h)             │
├─────────────────────────────────────┤
│   Zig core library                  │  ← Overlay logic lives here
├─────────────────────────────────────┤
│   SQLite .loaf file                 │  ← All writes captured here
├─────────────────────────────────────┤
│   Real filesystem (read-only)       │  ← Reads passthrough
└─────────────────────────────────────┘
```

## Implementation

1. **FSKit extension calls Zig library** for all filesystem operations
2. **Reads**: Check .loaf SQLite first, fall through to real filesystem if not found
3. **Writes**: Store in .loaf SQLite, NEVER touch real files
4. **Deletes**: Create whiteout marker in SQLite (hides real file)
5. **Mount/unmount**: `loaf mount <overlay.loaf> <mount-point>` and `loaf unmount <mount-point>`

## Commands

All commands use the FSKit overlay:

```bash
loaf run <cmd>    # Mount overlay → run cmd → unmount → accept/reject
loaf mount <path> # Mount overlay on directory
loaf unmount      # Unmount overlay
loaf diff         # Show changes in .loaf
loaf accept       # Apply .loaf changes to real filesystem
loaf reject       # Discard .loaf
```

## Overlay Semantics (for FSKit mode)

The SQLite "upper layer" captures all modifications:
- **Reads**: Check SQLite first, fall through to real filesystem if not found
- **Writes**: Store in SQLite, never touch real files
- **Deletes**: Create whiteout marker in SQLite (hides real file)
- **Renames**: Track path remapping in SQLite

This is copy-on-write at the filesystem level.

## Building

```bash
zig build                    # Build dynamic + static libraries
zig build test               # Run unit tests
zig build -Doptimize=ReleaseFast  # Release build
```

Output: `zig-out/lib/libloaf.dylib`, `zig-out/bin/loaf`

## Key Files

| File | Purpose |
|------|---------|
| `src/error.zig` | POSIX error codes + Zig error set |
| `src/db.zig` | SQLite schema and queries |
| `src/fs.zig` | Filesystem operations wrapper |
| `src/overlay_fs.zig` | Inode-based overlay operations |
| `src/overlay.zig` | Path-based overlay API |
| `src/main.zig` | C FFI exports |
| `src/cli.zig` | CLI tool |
| `include/loaf.h` | C header for Swift |

## SQLite Schema

Tables for overlay state:
- `inodes` - file metadata (type, mode, timestamps, parent_id)
- `file_data` - blob storage for modified/new file contents
- `xattrs` - extended attributes
- `whiteouts` - deleted file markers
- `overlay_config` - stores base_path for overlay mode

## CLI Commands

```bash
loaf mount <path>           # Mount overlay on directory
loaf unmount <path>         # Unmount overlay
loaf diff [path]            # Show pending changes
loaf accept [path]          # Apply changes to real filesystem
loaf reject [path]          # Discard all changes
loaf status                 # Show active mounts
```

## References

### Inspiration
- [poof](https://github.com/jarred-sumner/poof) - Ephemeral filesystem isolation for Linux (submodule at `deps/poof`)
- [agentfs](https://github.com/tursodatabase/agentfs) - SQLite-backed FS for agents by Turso

### FSKit
- [Apple FSKit Documentation](https://developer.apple.com/documentation/fskit)
- [FSKit Blog Post](https://eclecticlight.co/2024/06/26/how-file-systems-can-change-in-sequoia-with-fskit/)
- [FSKitSample](https://github.com/KhaosT/FSKitSample) - Example FSKit extension (submodule at `deps/FSKitSample`)

### Zig
- [zig-sqlite](https://github.com/vrischmann/zig-sqlite) - SQLite bindings (submodule at `deps/zig-sqlite`)

## Development Notes

### Zig 0.15 API Changes
- `std.ArrayList(T)` is now unmanaged - pass allocator to methods
- `b.addSharedLibrary` → `b.addLibrary(.{ .linkage = .dynamic })`
- zig-sqlite: use `oneAlloc`/`nextAlloc` for strings/blobs

### FFI Design
- C allocator only in FFI layer (stable ABI)
- Error codes map to POSIX errno values
- Opaque handles for Swift interop

### Swift 6 / FSKit API Notes (macOS 26)

**C Enum Interop:**
- C enums are imported as Swift structs, NOT Swift enums
- Must use `.rawValue` to compare: `result.rawValue == LOAF_OK.rawValue`
- Pass enum values directly to C functions: `LOAF_TYPE_FILE`, not `0`

**FSKit Type Names (differ from older docs):**
- `FSItem.Attributes` not `FSItemAttributes` (obsoleted)
- `FSItem.ItemType` values: `.file`, `.directory`, `.symlink` (not `.regular`, `.symbolicLink`)
- `FSVolume.CaseFormat`: `.insensitiveCasePreserving` (not `.caseSensitive` or `.sensitiveCasePreserving`)
- `FSItem.Identifier` initializer returns optional - must unwrap

**FSItem Subclass:**
- Do NOT override `isDirectory` - property doesn't exist on parent
- Store `attributes: FSItem.Attributes` as instance var, not computed property
- Populate attributes in `init()` or via `refreshAttributes()` method

**FSUnaryFileSystem Entry Point:**
- Use `UnaryFileSystemExtension` protocol, not `FSModuleExtension`
- Implement `var fileSystem: FSUnaryFileSystem & FSUnaryFileSystemOperations`

**FSResource Path Extraction:**
- When `FSSupportsPathURLs = true` in Info.plist, cast `FSResource` to `FSPathURLResource`
- Get file path via `pathResource.url.path`
- Example: `guard let pathResource = resource as? FSPathURLResource else { return }`

**timespec Conversion:**
- `tv_sec` expects `Int` not `Int64` - cast explicitly: `Int(attrs.atime_sec)`

**Build Configuration:**
- Must specify `ARCHS=arm64` if Zig library is arm64-only
- Deployment target must be >= 15.4 for FSKit
- Extension bundle ID must be prefixed with parent app bundle ID
