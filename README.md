<p align="center">
  <img src=".github/assets/header.svg" alt="loaf" width="100%"/>
</p>

<p align="center">
  <code>cargo install loaf && loaf run claude</code>
</p>

Overlay filesystem for macOS. Let AI modify your codebase freely, then accept or reject changes.

## How It Works

```
┌─────────────────────────────────────┐
│        Your commands / AI           │
├─────────────────────────────────────┤
│     Seatbelt Sandbox                │  ← Blocks writes outside overlay
├─────────────────────────────────────┤
│     NFS Server (userspace)          │  ← Intercepts all ops
├─────────────────────────────────────┤
│     SQLite overlay (.loaf)          │  ← All writes go here
├─────────────────────────────────────┤
│     Real filesystem (untouched)     │  ← Reads pass through
└─────────────────────────────────────┘
```

**Two layers of protection:**
1. **NFS overlay** - Copy-on-write semantics, all modifications stored in SQLite
2. **Seatbelt sandbox** - macOS kernel-level restriction, blocks writes outside the overlay

## Usage

```bash
cd ~/Projects/myapp
loaf run claude --dangerously-skip-permissions
```

When done, review the diff and choose: **y** to apply, **n** to discard.

```bash
loaf run <cmd> [args...]    # Run in sandbox
loaf run --no-sandbox <cmd> # Run without process sandbox (debugging)
loaf diff                   # Show pending changes
loaf accept                 # Apply changes
loaf reject                 # Discard changes
```

## Use Cases

- **AI agents**: Let Claude/GPT modify files freely, review before applying
- **Dangerous experiments**: `rm -rf ~` is harmless - blocked by sandbox
- **Package testing**: See what `npm install` actually touches

## Sandbox

The process sandbox uses macOS Seatbelt (same tech as App Sandbox) to restrict writes:

| Location | Read | Write |
|----------|------|-------|
| Overlay mount | ✓ | ✓ |
| `/tmp` | ✓ | ✓ |
| Everything else | ✓ | ✗ |

Network access is allowed (for `git`, `curl`, etc).

**Debug mode:** Set `LOAF_SANDBOX_DEBUG=1` to log denied operations:
```bash
LOAF_SANDBOX_DEBUG=1 loaf run bash
# In another terminal: log stream --predicate 'process == "sandboxd"'
```

## Status

Works via NFS userspace server. FSKit approach [blocked by Apple bugs](ISSUES.md).

---

<details>
<summary>Building from source</summary>

```bash
cargo build --release
```

Requires macOS 15+ and Rust 1.85+.

</details>
