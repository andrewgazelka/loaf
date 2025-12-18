<p align="center">
  <img src=".github/assets/header.svg" alt="loaf" width="100%"/>
</p>

**Fork your filesystem. Let AI run wild. Accept or reject changes.**

loaf is an overlay filesystem for macOS using FSKit. Like [poof](https://github.com/jarred-sumner/poof) for Linux, but native to macOS.

## How It Works

```
┌─────────────────────────────────────┐
│        Your commands / AI           │
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
zig build -Doptimize=ReleaseFast

# Let AI go wild in an isolated sandbox
cd ~/Projects/myapp
loaf run claude --dangerously-skip-permissions
```

```
● Creating sandbox...
● Running command in sandbox...

  ╭────────────────────────────────────────╮
  │  Claude Code v2.0.72                   │
  │  /private/tmp/loaf-sandbox-1766096819  │
  ╰────────────────────────────────────────╯

> make a file test.txt

● Write(test.txt)
  Wrote 1 lines to test.txt

> /exit

loaf: 1 changed file(s)
  + test.txt

Apply these changes to ~/Projects/myapp? [y/N/d(iff)]:
```

- **y** — Apply all changes to real filesystem
- **n** — Discard everything, directory unchanged

Uses FSKit to provide true filesystem isolation. All writes go to SQLite, real filesystem is never touched.

### Overlay Mode (Alternative)

```bash
# Create persistent overlay on a project
./zig-out/bin/loaf init ~/Projects/myapp

# Review what changed
./zig-out/bin/loaf diff

# Happy? Apply to real filesystem
./zig-out/bin/loaf accept

# Changed your mind? Discard everything
./zig-out/bin/loaf reject
```

## Use Cases

- **AI code agents**: Let Claude/GPT modify your codebase freely, review changes before applying
- **Dangerous experiments**: `rm -rf ~` without consequences
- **Package managers**: See what `npm install` actually touches before committing
- **Config changes**: Test system modifications with a safety net

## CLI Commands

```bash
# Sandbox execution (FSKit overlay)
loaf run <cmd> [args...]           # Run command in FSKit overlay sandbox

# Overlay mode
loaf init <path> [overlay.loaf]    # Create overlay on directory
loaf diff [overlay.loaf]           # Show pending changes
loaf accept [overlay.loaf]         # Apply changes to real filesystem
loaf reject [overlay.loaf]         # Discard all changes
loaf status                        # Show active overlay

# FSKit mount (requires enabled extension)
loaf mount <overlay.loaf> <mount>  # Mount .loaf file via FSKit
loaf unmount <mount>               # Unmount FSKit filesystem

# Inspection (debug)
loaf ls [db.loaf] [path]           # List directory contents
loaf cat [db.loaf] <path>          # Read file contents
loaf tree [db.loaf]                # Show full directory tree
loaf info [db.loaf]                # Show database stats
```

## Status

✅ **`loaf run` uses FSKit overlay** — true filesystem isolation via FSKit.

- [x] **`loaf run`** — sandbox execution with FSKit overlay
- [x] SQLite storage layer
- [x] Overlay database schema (whiteouts, path tracking)
- [x] `loaf init` — create overlay
- [x] `loaf diff` — show changes
- [x] `loaf accept` / `loaf reject` — apply or discard
- [x] FSKit extension structure
- [x] FSKit overlay integration (reads passthrough, writes to SQLite)
- [x] `loaf mount` / `loaf unmount` — mount overlay via FSKit

**Note:** Requires building and enabling the FSKit extension via Xcode.

## Building

```bash
# Build Zig library + CLI
zig build -Doptimize=ReleaseFast

# Run tests
zig build test

# Build Swift extension (requires Xcode)
cd swift && xcodegen generate && open Loaf.xcodeproj
```

## Architecture

```
┌─────────────────────────────┐
│   POSIX filesystem interface │
├─────────────────────────────┤
│   FSKit (Apple's framework)  │
├─────────────────────────────┤
│   Swift extension            │
├─────────────────────────────┤
│   C FFI bridge (loaf.h)      │
├─────────────────────────────┤
│   Zig core library           │
├─────────────────────────────┤
│   SQLite database            │  ← .loaf overlay file
└─────────────────────────────┘
```

The SQLite "upper layer" tracks:
- **Creates**: New files/directories stored as blobs
- **Writes**: Modified file contents
- **Deletes**: Whiteout markers hiding real files
- **Renames**: Path remapping

## Requirements

- macOS 15+ (Sequoia)
- Zig 0.15+
- Xcode 16+ (for FSKit extension)

## See Also

- [poof](https://github.com/jarred-sumner/poof) — Ephemeral filesystem isolation for Linux (inspiration)
- [agentfs](https://github.com/tursodatabase/agentfs) — SQLite-backed FS for agents by Turso
