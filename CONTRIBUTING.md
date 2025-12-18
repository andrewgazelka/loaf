# Contributing

## Prerequisites

- macOS 15+ (Sequoia)
- Zig 0.15+
- Xcode 16+
- Apple Developer account (for code signing)

## Development Setup

```bash
# Clone with submodules
git clone --recursive https://github.com/andrewgazelka/loaf.git
cd loaf

# Build Zig library
zig build

# Generate Xcode project
cd swift
xcodegen generate
open Loaf.xcodeproj
```

## Building

### Zig Core

```bash
# Debug build
zig build

# Release build
zig build -Doptimize=ReleaseFast

# Run Zig tests
zig build test
```

### Swift Extension

1. Open `swift/Loaf.xcodeproj` in Xcode
2. Select your development team for code signing
3. Build the `Loaf` target (Cmd+B)

## Testing

### Unit Tests (Zig)

```bash
zig build test
```

Tests the database layer and filesystem operations in isolation.

### Integration Tests

FSKit extensions require a signed app bundle. Integration testing involves:

1. **Build and install the app**:
   - Build the `Loaf` app in Xcode
   - Run the app once to register the extension

2. **Create a test database**:
   ```bash
   # The extension auto-creates the schema on first mount
   touch /tmp/test.loaf
   ```

3. **Mount the filesystem**:
   ```bash
   # FSKit filesystems are mounted via the system
   # After installing, .loaf files can be opened via Finder
   # or programmatically via FSKit APIs
   ```

4. **Run filesystem operations**:
   ```bash
   # Once mounted at /Volumes/test (example)
   cd /Volumes/test
   mkdir testdir
   echo "hello" > testdir/file.txt
   cat testdir/file.txt
   ls -la
   rm -r testdir
   ```

5. **Verify database contents**:
   ```bash
   sqlite3 /tmp/test.loaf "SELECT * FROM inodes;"
   ```

### Manual Testing Checklist

Before submitting a PR, verify:

- [ ] Create file: `touch /mount/file.txt`
- [ ] Write file: `echo "test" > /mount/file.txt`
- [ ] Read file: `cat /mount/file.txt`
- [ ] Create directory: `mkdir /mount/dir`
- [ ] List directory: `ls /mount/dir`
- [ ] Delete file: `rm /mount/file.txt`
- [ ] Delete directory: `rmdir /mount/dir`
- [ ] Symbolic link: `ln -s target /mount/link`
- [ ] Read symlink: `readlink /mount/link`
- [ ] Rename: `mv /mount/a /mount/b`
- [ ] Copy: `cp /mount/a /mount/b`

## Code Structure

```
loaf/
├── src/                    # Zig core
│   ├── main.zig           # C FFI exports
│   ├── fs.zig             # Filesystem operations
│   ├── db.zig             # SQLite schema & queries
│   └── error.zig          # Error types
├── include/
│   └── loaf.h             # C header for Swift
└── swift/
    ├── Loaf/              # Host app (minimal)
    └── LoafExtension/     # FSKit extension
        ├── LoafFileSystem.swift
        ├── LoafVolume.swift
        └── LoafItem.swift
```

## Making Changes

### Adding a New Filesystem Operation

1. Add the function to `src/fs.zig`
2. Export via C FFI in `src/main.zig`
3. Add declaration to `include/loaf.h`
4. Call from Swift in `LoafVolume.swift`
5. Add tests

### Modifying the Database Schema

1. Update `schema_sql` in `src/db.zig`
2. Add migration logic if needed
3. Update affected queries

## Commit Guidelines

- Atomic commits: one logical change per commit
- Clear messages: explain *why*, not just *what*
- No `--no-verify`: let pre-commit hooks run

## Pull Requests

1. Fork and create a feature branch
2. Make changes with tests
3. Run full test suite
4. Submit PR with description of changes
