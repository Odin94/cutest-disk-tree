import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { debugLog, findFiles, listCachedRoots } from "../api";
import { Button } from "../components/ui/button";
import type { FileSearchResult, ScanProgress, ScanResult } from "../types";
import { humanSize, progressTopLevelFolder } from "../utils";
import { FindResultsTable } from "./FileFindingView/FindResultsTable";

type TabId = "folders" | "files" | "duplicates" | "find";
export type FindSortKey = "name" | "size" | "path";
export type FindSortDirection = "asc" | "desc";
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

export const MAX_VISIBLE_FILES = 500;

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

export const getFileName = (path: string): string => {
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
  const [searchNextOffset, setSearchNextOffset] = useState<number | null>(null);
  const [searchLoadingMore, setSearchLoadingMore] = useState(false);
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
    category: FileCategory;
    fuzzy: boolean;
  } | null>(null);

  useEffect(() => {
    listCachedRoots().then(setCachedRoots);
  }, [result?.root]);

  useEffect(() => {
    lastQueryRef.current = null;
  }, [result?.root]);

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

  const PAGE_SIZE = MAX_VISIBLE_FILES;

  const runSearch = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!canSearch || result === null) return;
    const extension = getEffectiveExtensions();
    const queryKey = {
      root: result.root,
      query: searchQuery,
      extensions: extension,
      category: searchCategory,
      fuzzy: useFuzzySearch,
    };
    if (
      lastQueryRef.current &&
      lastQueryRef.current.root === queryKey.root &&
      lastQueryRef.current.query === queryKey.query &&
      lastQueryRef.current.extensions === queryKey.extensions &&
      lastQueryRef.current.category === queryKey.category &&
      lastQueryRef.current.fuzzy === queryKey.fuzzy
    ) {
      debugLog("FileFindingView runSearch skip (same query as current data)");
      return;
    }
    const reqT0 = performance.now();
    debugLog(
      `find_ui find_files request start root=${result.root} query_len=${searchQuery.length} ext=${extension.length > 0 ? "yes" : "no"} fuzzy=${useFuzzySearch ? "on" : "off"}`
    );
    setSearchLoading(true);
    setSearchLoadingMore(false);
    setSearchResults([]);
    setSearchNextOffset(null);
    setSearchTotalCount(null);
    setSearchError(null);
    try {
      const response = await findFiles(
        result.root,
        searchQuery,
        extension,
        useFuzzySearch,
        PAGE_SIZE,
        0
      );
      lastQueryRef.current = queryKey;
      const elapsed = (performance.now() - reqT0).toFixed(0);
      debugLog(
        `find_ui find_files response received count=${response.items.length} nextOffset=${response.nextOffset ?? "null"} elapsedMs=${elapsed}`
      );
      setSearchResults(response.items);
      setSearchNextOffset(response.nextOffset);
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
        category: searchCategory,
        fuzzy: useFuzzySearch,
      };
      if (
        lastQueryRef.current &&
        lastQueryRef.current.root === queryKey.root &&
        lastQueryRef.current.query === queryKey.query &&
        lastQueryRef.current.extensions === queryKey.extensions &&
        lastQueryRef.current.category === queryKey.category &&
        lastQueryRef.current.fuzzy === queryKey.fuzzy
      ) {
        debugLog("FileFindingView findFiles (debounced) skip (same query as current data)");
        return;
      }
      const reqT0 = performance.now();
      debugLog(
        `find_ui find_files request start (debounced) root=${result.root} query_len=${searchQuery.length} ext=${extension.length > 0 ? "yes" : "no"} fuzzy=${useFuzzySearch ? "on" : "off"}`
      );
      setSearchLoading(true);
      setSearchLoadingMore(false);
      setSearchResults([]);
      setSearchNextOffset(null);
      setSearchTotalCount(null);
      setSearchError(null);
      findFiles(result.root, searchQuery, extension, useFuzzySearch, PAGE_SIZE, 0)
        .then((response) => {
          lastQueryRef.current = queryKey;
          const elapsed = (performance.now() - reqT0).toFixed(0);
          debugLog(
            `find_ui find_files response received (debounced) count=${response.items.length} nextOffset=${response.nextOffset ?? "null"} elapsedMs=${elapsed}`
          );
          setSearchResults(response.items);
          setSearchNextOffset(response.nextOffset);
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
    searchCategory,
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
      visibleSearchResults: sorted,
      sortedCount: sorted.length,
    };
  }, [searchResults, searchCategory, findSortKey, findSortDirection]);

  const totalMatches =
    searchTotalCount != null ? searchTotalCount : sortedCount;

  const canLoadMore = searchNextOffset != null;

  const handleLoadMore = async () => {
    if (!canSearch || result === null) return;
    if (!canLoadMore) return;
    const extension = getEffectiveExtensions();
    const queryKey = {
      root: result.root,
      query: searchQuery,
      extensions: extension,
      category: searchCategory,
      fuzzy: useFuzzySearch,
    };
    const reqT0 = performance.now();
    debugLog(
      `find_ui find_files request load_more root=${result.root} query_len=${searchQuery.length} ext=${extension.length > 0 ? "yes" : "no"} fuzzy=${useFuzzySearch ? "on" : "off"} offset=${searchNextOffset}`
    );
    setSearchLoadingMore(true);
    try {
      const response = await findFiles(
        result.root,
        searchQuery,
        extension,
        useFuzzySearch,
        PAGE_SIZE,
        searchNextOffset ?? 0
      );
      lastQueryRef.current = queryKey;
      const elapsed = (performance.now() - reqT0).toFixed(0);
      debugLog(
        `find_ui find_files response received load_more count=${response.items.length} nextOffset=${response.nextOffset ?? "null"} elapsedMs=${elapsed}`
      );
      setSearchResults((prev) => [...prev, ...response.items]);
      setSearchNextOffset(response.nextOffset);
    } catch (e) {
      debugLog(
        `FileFindingView loadMore error: ${
          e instanceof Error ? e.message : String(e)
        }`
      );
      setSearchError(e instanceof Error ? e.message : String(e));
    } finally {
      setSearchLoadingMore(false);
    }
  };

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
                    {`${totalMatches.toLocaleString()} matching item${
                      totalMatches === 1 ? "" : "s"
                    }${canLoadMore ? " (more available…)" : ""}`}
                  </p>
                ) : null}
                <FindResultsTable
                  visibleSearchResults={visibleSearchResults}
                  searchLoading={searchLoading}
                  findSortKey={findSortKey}
                  findSortDirection={findSortDirection}
                  onToggleSort={toggleFindSort}
                  hasMore={canLoadMore}
                  loadingMore={searchLoadingMore}
                  onLoadMore={handleLoadMore}
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

  const renderMs = performance.now() - renderT0;
  if (renderMs > 10) {
    debugLog(
      `find_ui FileFindingView render slow ms=${renderMs.toFixed(1)} query_len=${searchQuery.length} results=${searchResults.length} loading=${searchLoading}`
    );
  }

  return view;
};
