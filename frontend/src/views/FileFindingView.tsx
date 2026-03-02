import { useState, useEffect } from "react";
import { findFiles, listCachedRoots, debugLog } from "../api";
import type { ScanResult, ScanProgress, FileSearchResult } from "../types";
import { humanSize, progressTopLevelFolder } from "../utils";
import { Button } from "../components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHeadCell,
  TableHeader,
  TableRow,
} from "../components/ui/table";

type TabId = "folders" | "files" | "duplicates" | "find";
type FindSortKey = "name" | "size" | "path";
type FindSortDirection = "asc" | "desc";
type FileCategory =
  | "all"
  | "audio"
  | "document"
  | "video"
  | "image"
  | "executable"
  | "compressed"
  | "config"
  | "folder"
  | "other";

type FileFindingViewProps = {
  result: ScanResult | null;
  loading: boolean;
  error: string | null;
  progress: ScanProgress | null;
  scanRootPath: string | null;
  onScan: () => void;
  onCancelScan?: () => void;
  onSelectCachedRoot?: (root: string) => Promise<void>;
};

const getFileName = (path: string): string => {
  const segments = path.split(/[/\\]/);
  if (segments.length === 0) return path;
  return segments[segments.length - 1] ?? path;
};

export const FileFindingView = ({
  result,
  loading,
  error,
  progress,
  scanRootPath,
  onScan,
  onCancelScan,
  onSelectCachedRoot,
}: FileFindingViewProps) => {
  const [activeTab, setActiveTab] = useState<TabId>("folders");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchExtensions, setSearchExtensions] = useState("");
  const [searchCategory, setSearchCategory] = useState<FileCategory>("all");
  const [searchResults, setSearchResults] = useState<FileSearchResult[]>([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [findSortKey, setFindSortKey] = useState<FindSortKey>("name");
  const [findSortDirection, setFindSortDirection] =
    useState<FindSortDirection>("asc");
  const [cachedRoots, setCachedRoots] = useState<string[]>([]);

  useEffect(() => {
    listCachedRoots().then(setCachedRoots);
  }, [result?.root]);

  const uniqueCount =
    result === null
      ? 0
      : new Set(
          result.files.map((f) => `${f.file_key.dev}:${f.file_key.ino}`)
        ).size;
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

  const getEffectiveExtensions = (): string => {
    const manual =
      searchExtensions.trim().length === 0
        ? []
        : searchExtensions
            .split(",")
            .map((raw) => raw.trim().replace(/^\./, ""))
            .filter((raw) => raw.length > 0);
    if (manual.length === 0) return "";
    const unique = Array.from(new Set(manual.map((ext) => ext.toLowerCase())));
    return unique.join(", ");
  };

  const runSearch = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!canSearch || result === null) return;
    debugLog(`FileFindingView runSearch root=${result.root} query=${searchQuery}`);
    setSearchLoading(true);
    setSearchError(null);
    try {
      const found = await findFiles(
        result.root,
        searchQuery,
        getEffectiveExtensions()
      );
      debugLog(`FileFindingView runSearch done count=${found.length}`);
      setSearchResults(found);
    } catch (e) {
      debugLog(
        `FileFindingView runSearch error: ${e instanceof Error ? e.message : String(e)}`
      );
      setSearchError(e instanceof Error ? e.message : String(e));
    } finally {
      setSearchLoading(false);
    }
  };

  useEffect(() => {
    if (!canSearch || result === null || activeTab !== "find") return;
    const timeoutId = window.setTimeout(() => {
      debugLog(`FileFindingView findFiles (debounced) root=${result.root} query=${searchQuery}`);
      setSearchLoading(true);
      setSearchError(null);
      findFiles(result.root, searchQuery, getEffectiveExtensions())
        .then((found) => {
          debugLog(`FileFindingView findFiles (debounced) done count=${found.length}`);
          setSearchResults(found);
        })
        .catch((e) => {
          const msg = e instanceof Error ? e.message : String(e);
          debugLog(`FileFindingView findFiles (debounced) error: ${msg}`);
          setSearchError(msg);
        })
        .finally(() => setSearchLoading(false));
    }, 120);
    return () => window.clearTimeout(timeoutId);
  }, [activeTab, canSearch, result?.root, searchQuery, searchExtensions]);

  const inferExtension = (path: string): string | null => {
    const name = getFileName(path);
    const idx = name.lastIndexOf(".");
    if (idx === -1 || idx === name.length - 1) return null;
    return name.slice(idx + 1).toLowerCase();
  };

  const audioExts = ["mp3", "wav", "flac", "m4a", "ogg", "aac", "opus"];
  const documentExts = [
    "pdf", "txt", "md", "rtf", "doc", "docx", "odt", "xls", "xlsx", "csv",
    "ppt", "pptx",
  ];
  const videoExts = ["mp4", "mkv", "mov", "avi", "webm", "m4v"];
  const imageExts = [
    "jpg", "jpeg", "png", "gif", "webp", "heic", "bmp", "tiff", "svg",
  ];
  const executableExts = [
    "exe", "dll", "so", "dylib", "bin", "sh", "bat", "cmd", "appimage",
  ];
  const compressedExts = ["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"];
  const configExts = [
    "cfg", "conf", "ini", "json", "yaml", "yml", "toml", "xml", "props",
    "properties", "rc", "config", "env",
  ];

  const classifyFileCategory = (item: FileSearchResult): FileCategory => {
    if (item.kind === "folder") return "folder";
    const ext = inferExtension(item.path);
    if (ext == null) return "other";
    if (audioExts.includes(ext)) return "audio";
    if (documentExts.includes(ext)) return "document";
    if (videoExts.includes(ext)) return "video";
    if (imageExts.includes(ext)) return "image";
    if (executableExts.includes(ext)) return "executable";
    if (compressedExts.includes(ext)) return "compressed";
    if (configExts.includes(ext)) return "config";
    return "other";
  };

  const filteredSearchResults = searchResults.filter((item) => {
    if (searchCategory === "all") return true;
    if (searchCategory === "folder") return item.kind === "folder";
    if (item.kind === "folder") return false;
    return classifyFileCategory(item) === searchCategory;
  });

  const sortedSearchResults = [...filteredSearchResults].sort((a, b) => {
    if (findSortKey === "size") {
      if (a.size === b.size) return 0;
      return findSortDirection === "asc" ? a.size - b.size : b.size - a.size;
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
      setFindSortDirection((d) => (d === "asc" ? "desc" : "asc"));
      return;
    }
    setFindSortKey(key);
    setFindSortDirection("asc");
  };

  const loadCached = async (root: string) => {
    if (onSelectCachedRoot) await onSelectCachedRoot(root);
  };

  return (
    <>
      <div className="file-finding-actions">
        <Button type="button" onClick={onScan} disabled={loading}>
          {loading ? "Scanning…" : "Choose folder to scan"}
        </Button>
        {cachedRoots.length > 1 ? (
          <select
            className="disk-usage-root-select"
            value={result?.root ?? ""}
            onChange={(e) => {
              const root = e.target.value;
              if (root) loadCached(root);
            }}
            title="Select previously scanned root"
          >
            <option value="">Select cached root…</option>
            {cachedRoots.map((root) => (
              <option key={root} value={root}>
                {root}
              </option>
            ))}
          </select>
        ) : null}
      </div>

      {error ? <div className="error">{error}</div> : null}

      {loading ? (
        <div className="progress-panel">
          <div
            className="progress-bar"
            role="progressbar"
            aria-valuenow={progress?.files_count ?? 0}
            aria-label="Scanning files"
          >
            <div className="progress-bar-inner" />
          </div>
          <p className="progress-text">
            {progress?.status != null
              ? progress.status
              : progress != null
                ? `${progress.files_count.toLocaleString()} files scanned…`
                : "Starting scan…"}
          </p>
          {progress != null ? (
            <>
              <p className="progress-text">
                {`${progress.files_count.toLocaleString()} files scanned`}
              </p>
              {progressTopLevelFolder(
                scanRootPath,
                progress.current_path ?? null
              ) != null ? (
                <p className="progress-path">
                  {`Top-level folder: ${progressTopLevelFolder(
                    scanRootPath,
                    progress.current_path ?? null
                  )}`}
                </p>
              ) : null}
              {progress.current_path != null ? (
                <p className="progress-path" title={progress.current_path}>
                  {progress.current_path}
                </p>
              ) : null}
            </>
          ) : null}
          {onCancelScan ? (
            <p className="progress-actions">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={onCancelScan}
              >
                Cancel scan
              </Button>
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
            {(["folders", "files", "duplicates", "find"] as const).map(
              (tab) => (
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
              )
            )}
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
                <p>
                  Hashing is not implemented yet; run from CLI to index, then
                  add hashing in a later step.
                </p>
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
                      <span className="find-label">Category</span>
                      <select
                        value={searchCategory}
                        onChange={(e) =>
                          setSearchCategory(e.target.value as FileCategory)
                        }
                        disabled={!canSearch}
                      >
                        <option value="all">All types</option>
                        <option value="audio">Audio</option>
                        <option value="document">Document</option>
                        <option value="video">Video</option>
                        <option value="image">Image</option>
                        <option value="executable">Executable</option>
                        <option value="compressed">Compressed</option>
                        <option value="config">Config</option>
                        <option value="folder">Folder</option>
                        <option value="other">Other</option>
                      </select>
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
                    Uses fuzzy matching (nucleo) against the indexed files for
                    this root.
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
                          key={`${f.path}:${f.file_key?.dev ?? "d"}:${f.file_key?.ino ?? "i"}`}
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
                          <TableCell
                            className="find-cell-path"
                            title={f.path}
                          >
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
    </>
  );
};
