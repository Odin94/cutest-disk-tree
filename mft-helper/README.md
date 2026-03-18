# mft-helper

A small stand-alone Windows binary that reads the **NTFS Master File Table (MFT)** for a drive and writes the result as JSON to a file.

## Why it exists

Reading the MFT directly is the fastest way to enumerate every file and folder on an NTFS volume — it skips the normal directory-walk entirely and instead reads the raw file-system index. The catch is that it requires **administrator privileges**.

Rather than making the main app request elevation at startup (which would show a UAC prompt every launch), the MFT scan is split into this helper binary. The main app launches it on demand via `Start-Process -Verb RunAs`, which triggers a one-off UAC prompt only when the user explicitly chooses the fast scan path.

## Usage

```
mft-helper <root-path> <output-file>
```

| Argument | Description |
|---|---|
| `root-path` | Drive or directory to scan, e.g. `C:\` |
| `output-file` | Path where JSON output will be written |

## Output format

```json
{
  "files": [
    { "path": "C:\\foo\\bar.txt", "size": 1234, "dev": 12345678, "ino": 9876543, "mtime": 1700000000 }
  ],
  "folders": [
    "C:\\foo",
    "C:\\foo\\baz"
  ]
}
```

`mtime` is a Unix timestamp (seconds) and may be `null` if unavailable. `dev` and `ino` are the Windows volume serial number and file index, used for deduplication.

## How it works

1. Opens the NTFS volume via `usn-journal-rs`.
2. **Pass 1** — iterates every MFT record, building a `fid → (parent_fid, name)` map for directories and collecting `(parent_fid, name)` for files.
3. **Pass 2** — reconstructs the full path of each file by walking up the directory map, then calls `fs::metadata` to get size and mtime.
4. Serialises the result to JSON and writes it to `output-file`.

Path reconstruction walks at most 256 levels deep to guard against cycles from corrupt MFT records.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Wrong number of arguments |
| 2 | MFT scan failed (non-NTFS volume, insufficient privileges, etc.) |
| 3 | Failed to serialise or write output file |

## Platform

Windows only. On other platforms the binary exits immediately with code 2.
