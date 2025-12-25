<p align="center">
  <img src=".github/assets/header.svg" alt="loaf" width="100%"/>
</p>

**Fork your filesystem. Let AI run wild. Accept or reject changes.**

loaf is an overlay filesystem for macOS. Like [poof](https://github.com/jarred-sumner/poof) for Linux, but for macOS.

## How It Works

```
┌─────────────────────────────────────┐
│        Your commands / AI           │
├─────────────────────────────────────┤
│     NFS Server (userspace)          │  ← Intercepts all operations
├─────────────────────────────────────┤
│     Overlay (SQLite upper layer)    │  ← All writes captured here
├─────────────────────────────────────┤
│     Real filesystem (read-only)     │  ← Reads pass through
└─────────────────────────────────────┘
```

Mount loaf over any directory. All changes go to a `.loaf` SQLite file. The real filesystem is untouched.

## Quick Start

```bash
# Build
cargo build --release

# Let AI go wild in an isolated sandbox
cd ~/Projects/myapp
loaf run claude --dangerously-skip-permissions
```

```
✓ Overlay mounted at /private/tmp/loaf-xxx/mount
  Running: claude --dangerously-skip-permissions

> make a file test.txt

● Write(test.txt)
  Wrote 1 lines to test.txt

> /exit

✓ Command completed successfully (exit code: 0)

Changes detected:
  A file     /test.txt

Apply changes to real filesystem? [y/N]:
```

- **y** — Apply all changes to real filesystem
- **n** — Discard everything, directory unchanged

## CLI Commands

```bash
# Sandbox execution (recommended)
loaf run <cmd> [args...]    # Run command in overlay sandbox

# Manual mount/unmount
loaf mount <path>           # Mount overlay on directory
loaf unmount <path>         # Unmount overlay

# Review changes
loaf diff [overlay.loaf]    # Show pending changes
loaf accept [overlay.loaf]  # Apply changes to real filesystem
loaf reject [overlay.loaf]  # Discard all changes
```

## Use Cases

- **AI code agents**: Let Claude/GPT modify your codebase freely, review changes before applying
- **Dangerous experiments**: `rm -rf ~` without consequences
- **Package managers**: See what `npm install` actually touches before committing
- **Config changes**: Test system modifications with a safety net

## Architecture

```
┌─────────────────────────────┐
│   POSIX filesystem calls    │
├─────────────────────────────┤
│   macOS mount_nfs           │
├─────────────────────────────┤
│   NFS server (nfsserve)     │
├─────────────────────────────┤
│   Overlay logic (Rust)      │
├─────────────────────────────┤
│   SQLite database           │  ← .loaf overlay file
└─────────────────────────────┘
```

The SQLite "upper layer" tracks:
- **Creates**: New files/directories stored as blobs
- **Writes**: Modified file contents
- **Deletes**: Whiteout markers hiding real files
- **Renames**: Path remapping

## Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test
```

## Requirements

- macOS (tested on 15+)
- Rust 1.85+ (edition 2024)

## Why NFS?

FSKit (Apple's new filesystem framework) has [known bugs on macOS 26](ISSUES.md) that prevent third-party extensions from working. NFS provides a reliable userspace alternative that works today.

## See Also

- [poof](https://github.com/jarred-sumner/poof) — Ephemeral filesystem isolation for Linux (inspiration)
- [agentfs](https://github.com/tursodatabase/agentfs) — SQLite-backed FS for agents by Turso
