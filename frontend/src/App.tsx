import { useState, useRef, useEffect } from "react";
import { scanDirectory, pickDirectory, onScanProgress, findFiles } from "./api";
import type { ScanResult, ScanProgress, FileEntry } from "./types";
import { humanSize } from "./utils";
import "./App.css";
import { Button } from "./components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHeadCell,
  TableHeader,
  TableRow,
} from "./components/ui/table";

const CheckForUpdatesButton = () => {
  const [checking, setChecking] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const check = async () => {
    setChecking(true);
    setMessage(null);
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();
      if (update != null) {
        setMessage(`Update ${update.version} available. Downloading…`);
        const { relaunch } = await import("@tauri-apps/plugin-process");
        await update.downloadAndInstall();
        setMessage("Update installed. Restarting…");
        await relaunch();
      } else {
        setMessage("No updates available.");
      }
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setChecking(false);
    }
  };

  return (
    <span className="updater-wrap">
      <button
        type="button"
        className="secondary"
        onClick={check}
        disabled={checking}
      >
        {checking ? "Checking…" : "Check for updates"}
      </button>
      {message != null ? (
        <span className="updater-message">{message}</span>
      ) : null}
    </span>
  );
};

type TabId = "folders" | "files" | "duplicates" | "find";
type FindSortKey = "name" | "size" | "path";
type FindSortDirection = "asc" | "desc";

const App = () => {
  const [result, setResult] = useState<ScanResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("folders");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchExtensions, setSearchExtensions] = useState("");
  const [searchResults, setSearchResults] = useState<FileEntry[]>([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [findSortKey, setFindSortKey] = useState<FindSortKey>("name");
  const [findSortDirection, setFindSortDirection] =
    useState<FindSortDirection>("asc");
  const unlistenRef = useRef<(() => void) | null>(null);

  const runScan = async () => {
    const path = await pickDirectory();
    if (path === null) return;
    setLoading(true);
    setProgress(null);
    setError(null);
    try {
      unlistenRef.current = await onScanProgress((p) => setProgress(p));
      const data = await scanDirectory(path);
      setResult(data);
      setActiveTab("folders");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
      setLoading(false);
      setProgress(null);
    }
  };

  const uniqueCount =
    result === null
      ? 0
      : new Set(result.files.map((f) => `${f.file_key.dev}:${f.file_key.ino}`)).size;
  const uniqueSize =
    result === null
      ? 0
      : (() => {
          const seen = new Set<string>();
          let sum = 0;
          for (const f of result.files) {
            const k = `${f.file_key.dev}:${f.file_key.ino}`;
            if (seen.has(k)) continue;
            seen.add(k);
            sum += f.size;
          }
          return sum;
        })();

  const folderList =
    result === null
      ? []
      : Object.entries(result.folder_sizes)
          .map(([path, size]) => ({ path, size }))
          .sort((a, b) => b.size - a.size);

  const largestFiles =
    result === null
      ? []
      : [...result.files]
          .sort((a, b) => b.size - a.size)
          .slice(0, 200);

  const canSearch = result !== null && !loading;

  const runSearch = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!canSearch || result === null) {
      return;
    }
    setSearchLoading(true);
    setSearchError(null);
    try {
      const found = await findFiles(
        result.root,
        searchQuery,
        searchExtensions
      );
      setSearchResults(found);
    } catch (e) {
      setSearchError(e instanceof Error ? e.message : String(e));
    } finally {
      setSearchLoading(false);
    }
  };

  useEffect(() => {
    if (!canSearch || result === null || activeTab !== "find") {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setSearchLoading(true);
      setSearchError(null);
      findFiles(result.root, searchQuery, searchExtensions)
        .then((found) => {
          setSearchResults(found);
        })
        .catch((e) => {
          setSearchError(e instanceof Error ? e.message : String(e));
        })
        .finally(() => {
          setSearchLoading(false);
        });
    }, 120);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [activeTab, canSearch, result, searchQuery, searchExtensions]);

  const getFileName = (path: string) => {
    const segments = path.split(/[/\\]/);
    if (segments.length === 0) {
      return path;
    }
    return segments[segments.length - 1] ?? path;
  };

  const sortedSearchResults = [...searchResults].sort((a, b) => {
    if (findSortKey === "size") {
      if (a.size === b.size) {
        return 0;
      }
      return findSortDirection === "asc"
        ? a.size - b.size
        : b.size - a.size;
    }

    if (findSortKey === "path") {
      const cmp = a.path.localeCompare(b.path);
      return findSortDirection === "asc" ? cmp : -cmp;
    }

    const nameA = getFileName(a.path);
    const nameB = getFileName(b.path);
    const cmp = nameA.localeCompare(nameB);
    return findSortDirection === "asc" ? cmp : -cmp;
  });

  const toggleFindSort = (key: FindSortKey) => {
    if (findSortKey === key) {
      setFindSortDirection(
        findSortDirection === "asc" ? "desc" : "asc"
      );
      return;
    }
    setFindSortKey(key);
    setFindSortDirection("asc");
  };

  return (
    <div className="app">
      <header className="header">
        <h1>Cutest Disk Tree</h1>
        <div className="header-actions">
          <Button type="button" onClick={runScan} disabled={loading}>
            {loading ? "Scanning…" : "Choose folder to scan"}
          </Button>
          <CheckForUpdatesButton />
        </div>
      </header>

      {error ? (
        <div className="error">{error}</div>
      ) : null}

      {loading ? (
        <div className="progress-panel">
          <div className="progress-bar" role="progressbar" aria-valuenow={progress?.files_count ?? 0} aria-label="Scanning files">
            <div className="progress-bar-inner" />
          </div>
          <p className="progress-text">
            {progress?.status != null
              ? progress.status
              : progress != null
                ? `${progress.files_count.toLocaleString()} files scanned…`
                : "Starting scan…"}
          </p>
          {progress?.current_path != null && progress?.status == null ? (
            <p className="progress-path" title={progress.current_path}>
              {progress.current_path}
            </p>
          ) : null}
        </div>
      ) : null}

      {result !== null && !loading ? (
        <>
          <section className="summary">
            <div className="summary-row">
              <span className="label">Root:</span>
              <span className="path">{result.root}</span>
            </div>
            <div className="summary-row">
              <span className="label">File entries:</span>
              <span>{result.files.length.toLocaleString()}</span>
            </div>
            <div className="summary-row">
              <span className="label">Unique files (hard links deduped):</span>
              <span>
                {uniqueCount.toLocaleString()} ({humanSize(uniqueSize)})
              </span>
            </div>
          </section>

          <div className="tabs">
            {(["folders", "files", "duplicates", "find"] as const).map((tab) => (
              <button
                key={tab}
                type="button"
                className={activeTab === tab ? "tab active" : "tab"}
                onClick={() => setActiveTab(tab)}
              >
                {tab === "folders"
                  ? "Largest folders"
                  : tab === "files"
                    ? "Largest files"
                    : tab === "duplicates"
                      ? "Duplicates"
                      : "Find files"}
              </button>
            ))}
          </div>

          <div className="panel">
            {activeTab === "folders" ? (
              <ul className="list folder-list">
                {folderList.slice(0, 100).map(({ path, size }) => (
                  <li key={path} className="list-item">
                    <span className="size">{humanSize(size)}</span>
                    <span className="path">{path}</span>
                  </li>
                ))}
              </ul>
            ) : activeTab === "files" ? (
              <ul className="list file-list">
                {largestFiles.map((f) => (
                  <li key={f.path} className="list-item">
                    <span className="size">{humanSize(f.size)}</span>
                    <span className="path">{f.path}</span>
                  </li>
                ))}
              </ul>
            ) : activeTab === "duplicates" ? (
              <div className="placeholder">
                <p>Duplicate detection will group files by content hash.</p>
                <p>Hashing is not implemented yet; run from CLI to index, then add hashing in a later step.</p>
              </div>
            ) : (
              <div className="find-panel">
                <form className="find-form" onSubmit={runSearch}>
                  <div className="find-fields">
                    <label className="find-field">
                      <span className="find-label">File name</span>
                      <input
                        type="text"
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        placeholder="e.g. package, main.tsx"
                        disabled={!canSearch}
                      />
                    </label>
                    <label className="find-field">
                      <span className="find-label">File endings</span>
                      <input
                        type="text"
                        value={searchExtensions}
                        onChange={(e) => setSearchExtensions(e.target.value)}
                        placeholder="e.g. ts, rs, .log"
                        disabled={!canSearch}
                      />
                    </label>
                    <Button
                      type="submit"
                      variant="secondary"
                      size="sm"
                      disabled={!canSearch || searchLoading}
                    >
                      {searchLoading ? "Searching…" : "Find files"}
                    </Button>
                  </div>
                  <p className="find-help">
                    Uses fuzzy matching (nucleo) against the indexed files for this root.
                  </p>
                </form>
                {searchError !== null ? (
                  <div className="error find-error">{searchError}</div>
                ) : null}
                <div className="find-table-wrap">
                  <Table className="find-table">
                    <TableHeader>
                      <TableRow>
                        <TableHeadCell>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="find-sort"
                            onClick={() => toggleFindSort("name")}
                          >
                            File name
                            {findSortKey === "name"
                              ? findSortDirection === "asc"
                                ? " ▲"
                                : " ▼"
                              : ""}
                          </Button>
                        </TableHeadCell>
                        <TableHeadCell>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="find-sort"
                            onClick={() => toggleFindSort("size")}
                          >
                            Size
                            {findSortKey === "size"
                              ? findSortDirection === "asc"
                                ? " ▲"
                                : " ▼"
                              : ""}
                          </Button>
                        </TableHeadCell>
                        <TableHeadCell>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="find-sort"
                            onClick={() => toggleFindSort("path")}
                          >
                            Path
                            {findSortKey === "path"
                              ? findSortDirection === "asc"
                                ? " ▲"
                                : " ▼"
                              : ""}
                          </Button>
                        </TableHeadCell>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {sortedSearchResults.map((f) => (
                        <TableRow
                          key={`${f.path}:${f.file_key.dev}:${f.file_key.ino}`}
                        >
                          <TableCell
                            className="find-cell-name"
                            title={getFileName(f.path)}
                          >
                            {getFileName(f.path)}
                          </TableCell>
                          <TableCell className="find-cell-size">
                            {humanSize(f.size)}
                          </TableCell>
                          <TableCell className="find-cell-path" title={f.path}>
                            {f.path}
                          </TableCell>
                        </TableRow>
                      ))}
                      {sortedSearchResults.length === 0 && !searchLoading ? (
                        <TableRow>
                          <TableCell className="find-empty" colSpan={3}>
                            No matches yet.
                          </TableCell>
                        </TableRow>
                      ) : null}
                    </TableBody>
                  </Table>
                </div>
              </div>
            )}
          </div>
        </>
      ) : loading ? null : (
        <div className="empty">
          Click &quot;Choose folder to scan&quot; to start.
        </div>
      )}
    </div>
  );
};

export default App;
