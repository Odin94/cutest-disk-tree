# cutest-disk-tree

Cross-platform disk usage and duplicate-file analyzer (Rust + Tauri + React + TypeScript).

## CLI indexer (prototype)

Walk a directory tree, aggregate folder sizes, and avoid double-counting hard links.

**Build** (requires [Rust](https://rustup.rs)):

```bash
cargo build
```

**Run**:

```bash
cargo run -- <path>
# or
cargo run -- .
```

- **Symlinks**: Not followed (ignored for traversal).
- **Hard links**: Counted once per (device, inode) on Unix; per (volume, file id) on Windows.
- Output: Total file entries, unique file count/size, and top 20 folders by recursive size.
