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
│     NFS Server (userspace)          │  ← Intercepts all ops
├─────────────────────────────────────┤
│     SQLite overlay (.loaf)          │  ← All writes go here
├─────────────────────────────────────┤
│     Real filesystem (untouched)     │  ← Reads pass through
└─────────────────────────────────────┘
```

## Usage

```bash
cd ~/Projects/myapp
loaf run claude --dangerously-skip-permissions
```

When done, review the diff and choose: **y** to apply, **n** to discard.

```bash
loaf run <cmd> [args...]    # Run in sandbox
loaf diff                   # Show pending changes
loaf accept                 # Apply changes
loaf reject                 # Discard changes
```

## Use Cases

- **AI agents**: Let Claude/GPT go wild, review before applying
- **Dangerous experiments**: `rm -rf ~` without consequences
- **Package testing**: See what `npm install` actually touches

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
