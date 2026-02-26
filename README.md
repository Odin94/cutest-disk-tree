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
- **Check for updates**: Uses `tauri-plugin-updater`; it fetches [latest.json](https://github.com/Odin94/cutest-disk-tree/releases/latest/download/latest.json) from this repo’s releases. For production builds use `./scripts/release.sh`, which sets the updater pubkey from `.tauri-public-key` (see [Releasing](#releasing-github)).

Scan results are stored in SQLite in the app data directory (`index.db`). Each scan overwrites data for that root path; you can re-scan to refresh.

**Database location** (app identifier `com.cutest.disk-tree`):

| OS | Path |
|----|------|
| **Windows** | `%APPDATA%\com.cutest.disk-tree\index.db` (e.g. `C:\Users\<You>\AppData\Roaming\com.cutest.disk-tree\index.db`) |
| **macOS** | `~/Library/Application Support/com.cutest.disk-tree/index.db` |
| **Linux** | `~/.local/share/com.cutest.disk-tree/index.db` (or `$XDG_DATA_HOME/com.cutest.disk-tree/index.db` if set) |

**Build for production**:

```bash
npm run tauri build
```

## Releasing (GitHub)

Releases are published to [Odin94/cutest-disk-tree](https://github.com/Odin94/cutest-disk-tree). The in-app “Check for updates” uses `latest.json` from the latest release.

### One-time setup (signing)

1. Generate a key pair (keep the private key secret and backed up):
   ```bash
   npm run tauri signer generate -w ~/.tauri/cutest-disk-tree.key
   ```
2. Save the **private** key as `.tauri-private-key` and the **public** key as `.tauri-public-key` in the repo root (base64-encoded). The release script reads these and sets `plugins.updater.pubkey` in `tauri.conf.json` at build time. Keep `.tauri-private-key` out of version control (it is in `.gitignore`).

### For each release (recommended: use the script)

1. Bump `version` in `src-tauri/tauri.conf.json` (and `package.json` if you use it for display).
2. Either:
   - **Build and release in one go:** Run `./scripts/build-all-platforms.sh` (Bash). It will confirm pre-steps, run a signed build, then run the release script to produce `release-out/v<VERSION>/latest.json` and `manifest.txt`.
   - **Or build yourself, then release:** Run a signed build, then run `./scripts/release.sh`. It will patch the updater config, then generate `latest.json` and `manifest.txt` from the existing bundle. It does **not** upload anything; it prints the manual GitHub steps at the end.
3. Follow the printed steps: create the release and tag on GitHub, upload the files listed in `manifest.txt` (installers, `.sig` files, and `latest.json`), then publish.

**Multi-platform:** The script produces a `latest.json` for the platform you built on. For updates on multiple OSes, run the build (and release script) on each OS and merge the `platforms` blocks from each `latest.json` into one, then upload that single `latest.json` to the release.

### For each release (manual)

If you prefer not to use the script:
   ```bash
   $env:TAURI_SIGNING_PRIVATE_KEY="<path-or-content-of-private-key>"
   npm run tauri build
   ```
   On macOS/Linux use `export TAURI_SIGNING_PRIVATE_KEY="..."` instead.
3. Open [Releases](https://github.com/Odin94/cutest-disk-tree/releases) → “Draft a new release”.
4. Create a tag (e.g. `v0.1.1`) and publish the release.
5. Upload the built artifacts from `src-tauri/target/release/bundle/` and add `latest.json` as described in the script’s output (or in the “Check for updates” paragraph below).
6. Publish the release.

After that, existing installs will see the update when users click “Check for updates”.

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
