# Contributing

## Prerequisites

- macOS 15+
- Rust 1.85+ (edition 2024)

## Development Setup

```bash
git clone https://github.com/andrewgazelka/loaf.git
cd loaf
cargo build
```

## Building

```bash
cargo build              # Debug
cargo build --release    # Release
cargo test               # Run tests
```

## Testing

### Unit Tests

```bash
cargo test
```

### Manual Testing

```bash
# Create a test directory
mkdir /tmp/test-project
cd /tmp/test-project
echo "hello" > file.txt

# Run loaf
loaf run bash

# Inside the sandbox, make changes
echo "modified" > file.txt
rm file.txt
mkdir newdir
exit

# Review and accept/reject changes
```

### Testing Checklist

Before submitting a PR:

- [ ] `loaf run <cmd>` mounts and unmounts cleanly
- [ ] File operations work (create, read, write, delete)
- [ ] Directory operations work (mkdir, rmdir, ls)
- [ ] Symlinks work
- [ ] `loaf diff` shows correct changes
- [ ] `loaf accept` applies changes to real filesystem
- [ ] `loaf reject` discards overlay cleanly

## Code Structure

```
loaf/
├── src/
│   ├── main.rs      # CLI entry point
│   ├── lib.rs       # Library exports
│   ├── db.rs        # SQLite schema & queries
│   ├── nfs.rs       # NFS server implementation
│   └── overlay.rs   # Overlay filesystem logic
└── tests/           # Integration tests
```

## Making Changes

### Adding a New CLI Command

1. Add variant to `Commands` enum in `src/main.rs`
2. Implement handler function
3. Add to match statement in `main()`

### Adding a Filesystem Operation

1. Add method to `OverlayFs` in `src/overlay.rs`
2. Implement NFS handler in `src/nfs.rs`
3. Add tests

### Modifying the Database Schema

1. Update schema in `src/db.rs`
2. Add migration if needed
3. Update affected queries

## Commit Guidelines

- Atomic commits: one logical change per commit
- Clear messages: explain *why*, not just *what*
- Run `cargo fmt` and `cargo clippy` before committing

## Pull Requests

1. Fork and create a feature branch
2. Make changes with tests
3. Run `cargo test`
4. Submit PR with description of changes
