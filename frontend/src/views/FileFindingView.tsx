import { useState, useEffect, useRef, useMemo, memo, useCallback } from "react";
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

const MAX_VISIBLE_FILES = 500;

type FindResultsTableProps = {
  visibleSearchResults: FileSearchResult[];
  totalMatches: number;
  searchLoading: boolean;
  findSortKey: FindSortKey;
  findSortDirection: FindSortDirection;
  onToggleSort: (key: FindSortKey) => void;
};

const FindResultsTable = memo(({
  visibleSearchResults,
  totalMatches,
  searchLoading,
  findSortKey,
  findSortDirection,
  onToggleSort,
}: FindResultsTableProps) => {
  const renderT0 = performance.now();
  debugLog(`find_ui FindResultsTable render start rows=${visibleSearchResults.length}`);
  const out = (
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
                onClick={() => {
                  debugLog("FileFindingView click sort name");
                  onToggleSort("name");
                }}
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
                onClick={() => {
                  debugLog("FileFindingView click sort size");
                  onToggleSort("size");
                }}
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
                onClick={() => {
                  debugLog("FileFindingView click sort path");
                  onToggleSort("path");
                }}
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
          {visibleSearchResults.map((f) => (
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
          {visibleSearchResults.length === 0 && !searchLoading ? (
            <TableRow>
              <TableCell className="find-empty" colSpan={3}>
                No matches yet.
              </TableCell>
            </TableRow>
          ) : null}
          {totalMatches > MAX_VISIBLE_FILES ? (
            <TableRow>
              <TableCell className="find-status-bottom" colSpan={3}>
                {`Showing first ${MAX_VISIBLE_FILES.toLocaleString()} matches out of ${totalMatches.toLocaleString()} total.`}
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
    </div>
  );
  debugLog(`find_ui FindResultsTable render done ms=${(performance.now() - renderT0).toFixed(1)}`);
  return out;
});
FindResultsTable.displayName = "FindResultsTable";

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
  const renderT0 = performance.now();
  const [activeTab, setActiveTab] = useState<TabId>("find");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchExtensions, setSearchExtensions] = useState("");
  const [searchCategory, setSearchCategory] = useState<FileCategory>("all");
  const [useFuzzySearch, setUseFuzzySearch] = useState(false);
  const [searchResults, setSearchResults] = useState<FileSearchResult[]>([]);
  const [searchTotalCount, setSearchTotalCount] = useState<number | null>(null);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [findSortKey, setFindSortKey] = useState<FindSortKey>("name");
  const [findSortDirection, setFindSortDirection] =
    useState<FindSortDirection>("asc");
  const [cachedRoots, setCachedRoots] = useState<string[]>([]);
  const lastQueryRef = useRef<{
    root: string;
    query: string;
    extensions: string;
  } | null>(null);

  useEffect(() => {
    listCachedRoots().then(setCachedRoots);
  }, [result?.root]);

  useEffect(() => {
    lastQueryRef.current = null;
  }, [result?.root]);

  debugLog(
    `find_ui FileFindingView render start query_len=${searchQuery.length} results=${searchResults.length} loading=${searchLoading}`
  );

  const { uniqueCount, uniqueSize, folderList, largestFiles } = useMemo(() => {
    if (result === null) {
      return {
        uniqueCount: 0,
        uniqueSize: 0,
        folderList: [] as { path: string; size: number }[],
        largestFiles: [] as FileSearchResult[],
      };
    }

    const t0 = performance.now();

    const seen = new Set<string>();
    let sum = 0;
    for (const f of result.files) {
      const k = `${f.file_key.dev}:${f.file_key.ino}`;
      if (seen.has(k)) continue;
      seen.add(k);
      sum += f.size;
    }

    const folders = Object.entries(result.folder_sizes)
      .map(([path, size]) => ({ path, size }))
      .sort((a, b) => b.size - a.size);

    const largest = [...result.files]
      .sort((a, b) => b.size - a.size)
      .slice(0, MAX_VISIBLE_FILES);

    const ms = (performance.now() - t0).toFixed(1);
    debugLog(
      `find_ui summary_compute ms=${ms} files=${result.files.length} folders=${folders.length}`
    );

    return {
      uniqueCount: seen.size,
      uniqueSize: sum,
      folderList: folders,
      largestFiles: largest,
    };
  }, [result]);

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
    const extension = getEffectiveExtensions();
    const queryKey = { root: result.root, query: searchQuery, extensions: extension };
    if (
      lastQueryRef.current &&
      lastQueryRef.current.root === queryKey.root &&
      lastQueryRef.current.query === queryKey.query &&
      lastQueryRef.current.extensions === queryKey.extensions
    ) {
      debugLog("FileFindingView runSearch skip (same query as current data)");
      return;
    }
    const reqT0 = performance.now();
    debugLog(
      `find_ui find_files request start root=${result.root} query_len=${searchQuery.length} ext=${extension.length > 0 ? "yes" : "no"} fuzzy=${useFuzzySearch ? "on" : "off"}`
    );
    setSearchLoading(true);
    setSearchError(null);
    try {
      const found = await findFiles(
        result.root,
        searchQuery,
        extension,
        useFuzzySearch
      );
      lastQueryRef.current = queryKey;
      const elapsed = (performance.now() - reqT0).toFixed(0);
      debugLog(`find_ui find_files response received count=${found.length} elapsedMs=${elapsed}`);
      setSearchTotalCount(found.length);
      const limited =
        found.length > MAX_VISIBLE_FILES
          ? found.slice(0, MAX_VISIBLE_FILES)
          : found;
      setSearchResults(limited);
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
      const extension = getEffectiveExtensions();
      const queryKey = {
        root: result.root,
        query: searchQuery,
        extensions: extension,
      };
      if (
        lastQueryRef.current &&
        lastQueryRef.current.root === queryKey.root &&
        lastQueryRef.current.query === queryKey.query &&
        lastQueryRef.current.extensions === queryKey.extensions
      ) {
        debugLog("FileFindingView findFiles (debounced) skip (same query as current data)");
        return;
      }
      const reqT0 = performance.now();
      debugLog(
        `find_ui find_files request start (debounced) root=${result.root} query_len=${searchQuery.length} ext=${extension.length > 0 ? "yes" : "no"} fuzzy=${useFuzzySearch ? "on" : "off"}`
      );
      setSearchLoading(true);
      setSearchError(null);
      findFiles(result.root, searchQuery, extension, useFuzzySearch)
        .then((found) => {
          lastQueryRef.current = queryKey;
          const elapsed = (performance.now() - reqT0).toFixed(0);
          debugLog(`find_ui find_files response received (debounced) count=${found.length} elapsedMs=${elapsed}`);
          setSearchTotalCount(found.length);
          const limited =
            found.length > MAX_VISIBLE_FILES
              ? found.slice(0, MAX_VISIBLE_FILES)
              : found;
          setSearchResults(limited);
        })
        .catch((e) => {
          const msg = e instanceof Error ? e.message : String(e);
          debugLog(`FileFindingView findFiles (debounced) error: ${msg}`);
          setSearchError(msg);
        })
        .finally(() => setSearchLoading(false));
    }, 200);
    return () => window.clearTimeout(timeoutId);
  }, [
    activeTab,
    canSearch,
    result?.root,
    searchQuery,
    searchExtensions,
    useFuzzySearch,
  ]);

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

  const { visibleSearchResults, sortedCount } = useMemo(() => {
    const filtered = searchResults.filter((item) => {
      if (searchCategory === "all") return true;
      if (searchCategory === "folder") return item.kind === "folder";
      if (item.kind === "folder") return false;
      return classifyFileCategory(item) === searchCategory;
    });
    const sorted = [...filtered].sort((a, b) => {
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
    return {
      visibleSearchResults: sorted.slice(0, MAX_VISIBLE_FILES),
      sortedCount: sorted.length,
    };
  }, [searchResults, searchCategory, findSortKey, findSortDirection]);

  const totalMatches =
    searchTotalCount != null ? searchTotalCount : sortedCount;

  const toggleFindSort = useCallback((key: FindSortKey) => {
    setFindSortKey((current) => {
      if (current === key) {
        setFindSortDirection((d) => (d === "asc" ? "desc" : "asc"));
        return current;
      }
      setFindSortDirection("asc");
      return key;
    });
  }, []);

  const loadCached = async (root: string) => {
    if (onSelectCachedRoot) await onSelectCachedRoot(root);
  };

  const view = (
    <>
      <div className="file-finding-actions">
        <Button
          type="button"
          onClick={() => {
            debugLog("FileFindingView click Choose folder to scan");
            onScan();
          }}
          disabled={loading}
        >
          {loading ? "Scanning…" : "Choose folder to scan"}
        </Button>
        {cachedRoots.length > 1 ? (
          <select
            className="disk-usage-root-select"
            value={result?.root ?? ""}
            onChange={(e) => {
              const root = e.target.value;
              debugLog(`FileFindingView select root value=${root || "(empty)"}`);
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
                onClick={() => {
                  debugLog("FileFindingView click Cancel scan");
                  onCancelScan();
                }}
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
            {(["find", "folders", "files", "duplicates"] as const).map(
              (tab) => (
                <button
                  key={tab}
                  type="button"
                  className={activeTab === tab ? "tab active" : "tab"}
                  onClick={() => {
                    const label =
                      tab === "folders"
                        ? "Largest folders"
                        : tab === "files"
                          ? "Largest files"
                          : tab === "duplicates"
                            ? "Duplicates"
                            : "Find files";
                    debugLog(`FileFindingView click tab ${tab} (${label})`);
                    setActiveTab(tab);
                  }}
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
              <div className="file-list-wrap">
                <ul className="list file-list">
                  {largestFiles.map((f) => (
                    <li key={f.path} className="list-item">
                      <span className="size">{humanSize(f.size)}</span>
                      <span className="path">{f.path}</span>
                    </li>
                  ))}
                </ul>
                {result.files.length > MAX_VISIBLE_FILES ? (
                  <p className="list-status">
                    {`Showing largest ${MAX_VISIBLE_FILES.toLocaleString()} files out of ${result.files.length.toLocaleString()} total.`}
                  </p>
                ) : null}
              </div>
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
                        onChange={(e) => {
                          const v = e.target.value;
                          debugLog(`FileFindingView input file_name value=${JSON.stringify(v)}`);
                          setSearchQuery(v);
                        }}
                        placeholder="e.g. package, main.tsx"
                        disabled={!canSearch}
                      />
                    </label>
                    <label className="find-field">
                      <span className="find-label">Category</span>
                      <select
                        value={searchCategory}
                        onChange={(e) => {
                          const v = e.target.value as FileCategory;
                          debugLog(`FileFindingView select category value=${v}`);
                          setSearchCategory(v);
                        }}
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
                        onChange={(e) => {
                          const v = e.target.value;
                          debugLog(`FileFindingView input file_endings value=${JSON.stringify(v)}`);
                          setSearchExtensions(v);
                        }}
                        placeholder="e.g. ts, rs, .log"
                        disabled={!canSearch}
                      />
                    </label>
                    <label className="find-field">
                      <span className="find-label">Fuzzy search</span>
                      <input
                        type="checkbox"
                        checked={useFuzzySearch}
                        onChange={(e) => {
                          const checked = e.target.checked;
                          debugLog(
                            `FileFindingView toggle fuzzy_search value=${JSON.stringify(
                              checked
                            )}`
                          );
                          setUseFuzzySearch(checked);
                        }}
                        disabled={!canSearch}
                      />
                    </label>
                    <Button
                      type="submit"
                      variant="secondary"
                      size="sm"
                      disabled={!canSearch || searchLoading}
                      onClick={() => debugLog("FileFindingView click Find files submit")}
                    >
                      {searchLoading ? "Searching…" : "Find files"}
                    </Button>
                  </div>
                  <p className="find-help">
                    Uses fast substring matching by default; enable fuzzy search
                    to use nucleo-based fuzzy matching against the indexed files
                    for this root.
                  </p>
                </form>
                {searchError !== null ? (
                  <div className="error find-error">{searchError}</div>
                ) : null}
                {searchLoading ? (
                  <p className="find-status">Searching files…</p>
                ) : totalMatches > 0 ? (
                  <p className="find-status">
                    {`${totalMatches.toLocaleString()} matching item${totalMatches === 1 ? "" : "s"
                      }${totalMatches > MAX_VISIBLE_FILES
                        ? ` (showing first ${MAX_VISIBLE_FILES.toLocaleString()})`
                        : ""
                      }`}
                  </p>
                ) : null}
                <FindResultsTable
                  visibleSearchResults={visibleSearchResults}
                  totalMatches={totalMatches}
                  searchLoading={searchLoading}
                  findSortKey={findSortKey}
                  findSortDirection={findSortDirection}
                  onToggleSort={toggleFindSort}
                />
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

  debugLog(
    `find_ui FileFindingView render done ms=${(performance.now() - renderT0).toFixed(1)}`
  );

  return view;
};
