# File-updating strategy

Three complementary mechanisms keep the in-memory `TrigramIndex` in sync with the filesystem. Together they cover real-time changes, offline changes, and the gap between rescans.

---

## 1. Full rescan (source of truth)

Triggered manually by the user (or on first launch). Walks the entire disk, rebuilds the index from scratch, and persists everything to SQLite. All other mechanisms work *on top of* this baseline.

## 2. Real-time OS watcher (`IndexWatcher`)

Starts after a rescan. Registers with the OS kernel (`ReadDirectoryChangesW` on Windows, `FSEvents` on macOS, `inotify` on Linux) so the kernel pushes a notification whenever a file is created, deleted, or renamed — **no polling, negligible CPU**.

**Limitation:** only active while the program is running. Any changes that happen while the program is closed are invisible to it.

## 3. Background reconciler (`IndexReconciler`)

Starts at launch if no rescan is already in progress. Walks the entire disk *slowly* (a small sleep between every entry, single thread, minimal stack) and patches the index for anything that changed while the program wasn't running:

- **New file on disk, missing from index** → `index.add()`
- **Path in index, gone from disk** → `index.remove()`

The reconciler **pauses automatically** whenever a full rescan is running and restarts its walk from scratch once the scan finishes, so it always works from a clean baseline.

One full pass takes several minutes on a typical disk — this is intentional. The reconciler is meant to be invisible, not fast.

---

## Summary

| Mechanism | Latency | Covers offline changes | Resource cost |
|-----------|---------|----------------------|---------------|
| Full rescan | minutes | yes | high (user-triggered) |
| OS watcher | milliseconds | no | ~zero |
| Background reconciler | minutes | yes | ~zero |
