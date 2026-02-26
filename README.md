# cutest-disk-tree

Cross-platform disk usage and duplicate-file analyzer (Rust + Tauri + React + TypeScript).

## Tauri app (React UI)

**Prerequisites**: [Rust](https://rustup.rs), [Node.js](https://nodejs.org).

**First-time setup** (generates app icons if missing):

```bash
npm install
node scripts/gen-icon.cjs
```

**Run the app**:

```bash
npm run tauri dev
```

- **Choose folder to scan**: Opens the native directory picker, then runs the indexer.
- **Summary**: Root path, file entry count, unique files (hard links deduped) and total size.
- **Largest folders**: Top 100 folders by recursive size.
- **Largest files**: Top 200 files by size.
- **Duplicates**: Placeholder (hashing not implemented yet).

**Build for production**:

```bash
npm run tauri build
```

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
