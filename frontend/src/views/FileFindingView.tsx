import { useEffect, useMemo, useRef, useState } from "react";
import { motion } from "framer-motion";
import { debugLog, findFiles } from "../api";
import { Button } from "../components/ui/button";
import { IndexingControls } from "../components/file-finding/IndexingControls";
import SearchBar from "../components/file-finding/SearchBar";
import { FilterBar } from "../components/file-finding/FilterBar";
import { FileTable } from "../components/file-finding/FileTable";
import { FolderList } from "../components/file-finding/FolderList";
import type { FileSearchResult, ScanProgress, ScanResult } from "../types";
import cozyBg from "../assets/cozy-bg.jpg";

export type TabId = "find" | "folders";
type FileCategory = "all" | "audio" | "document" | "video" | "image" | "executable" | "compressed" | "config" | "folder" | "other";

export const MAX_VISIBLE_FILES = 500;

type FileFindingViewProps = {
  result: ScanResult | null;
  loading: boolean;
  error: string | null;
  progress: ScanProgress | null;
  scanPhaseStatus?: string;
  onScan: () => void;
  onCancelScan?: () => void;
  activeTab: TabId;
  onTabChange: (tab: TabId) => void;
};

export const getFileName = (path: string): string => {
  const segments = path.split(/[/\\]/);
  return segments[segments.length - 1] ?? path;
};

export const FileFindingView = ({ result, loading, error, progress, scanPhaseStatus = "", onScan, onCancelScan, activeTab }: FileFindingViewProps) => {
  const [searchQuery, setSearchQuery] = useState("");
  const [searchExtensions, setSearchExtensions] = useState("");
  const [searchCategory, setSearchCategory] = useState<FileCategory>("all");
  const [useFuzzySearch, setUseFuzzySearch] = useState(false);
  const [searchResults, setSearchResults] = useState<FileSearchResult[]>([]);
  const [searchNextOffset, setSearchNextOffset] = useState<number | null>(null);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchLoadingMore, setSearchLoadingMore] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const lastQueryRef = useRef<{
    query: string;
    extensions: string;
    category: FileCategory;
    fuzzy: boolean;
  } | null>(null);

  useEffect(() => {
    lastQueryRef.current = null;
  }, [result]);

  const { filesCount, folderList } = useMemo(() => {
    if (result === null) {
      return { filesCount: 0, folderList: [] as { path: string; size: number }[] };
    }
    const filesCount = result.files_count ?? result.files.length;
    const folders = Object.entries(result.folder_sizes)
      .map(([path, size]) => ({ path, size }))
      .sort((a, b) => b.size - a.size);
    return { filesCount, folderList: folders };
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
    return Array.from(new Set(manual.map((ext) => ext.toLowerCase()))).join(", ");
  };

  const PAGE_SIZE = MAX_VISIBLE_FILES;

  useEffect(() => {
    if (!canSearch || result === null || activeTab !== "find") return;
    const timeoutId = window.setTimeout(() => {
      const extensions = getEffectiveExtensions();
      const queryKey = {
        query: searchQuery,
        extensions,
        category: searchCategory,
        fuzzy: useFuzzySearch,
      };
      if (
        lastQueryRef.current &&
        lastQueryRef.current.query === queryKey.query &&
        lastQueryRef.current.extensions === queryKey.extensions &&
        lastQueryRef.current.category === queryKey.category &&
        lastQueryRef.current.fuzzy === queryKey.fuzzy
      ) {
        return;
      }
      setSearchLoading(true);
      setSearchResults([]);
      setSearchNextOffset(null);
      setSearchError(null);
      const t0 = performance.now();
      debugLog(`find_files invoke_start query_len=${searchQuery.length} category=${searchCategory}`);
      findFiles(searchQuery, extensions, searchCategory, useFuzzySearch, PAGE_SIZE, 0)
        .then((response) => {
          const ipcMs = Math.round(performance.now() - t0);
          debugLog(`find_files ipc_done ipc_total_ms=${ipcMs} items=${response.items.length} has_more=${response.nextOffset != null}`);
          lastQueryRef.current = queryKey;
          setSearchResults(response.items);
          setSearchNextOffset(response.nextOffset);
        })
        .catch((e) => setSearchError(e instanceof Error ? e.message : String(e)))
        .finally(() => setSearchLoading(false));
    }, 200);
    return () => window.clearTimeout(timeoutId);
  }, [activeTab, canSearch, searchQuery, searchExtensions, searchCategory, useFuzzySearch]);

  const canLoadMore = searchNextOffset != null;

  const handleLoadMore = async () => {
    if (!canSearch || result === null || !canLoadMore || searchLoadingMore) return;
    setSearchLoadingMore(true);
    try {
      const response = await findFiles(
        searchQuery,
        getEffectiveExtensions(),
        searchCategory,
        useFuzzySearch,
        PAGE_SIZE,
        searchNextOffset ?? 0
      );
      setSearchResults((prev) => [...prev, ...response.items]);
      setSearchNextOffset(response.nextOffset);
    } catch (e) {
      setSearchError(e instanceof Error ? e.message : String(e));
    } finally {
      setSearchLoadingMore(false);
    }
  };

  const view = (
    <div className="min-h-screen relative overflow-hidden cozy-file-finding">
      <div className="fixed inset-0 -z-10">
        <img src={cozyBg} alt="" className="w-full h-full object-cover" />
        <div className="absolute inset-0 bg-background/40 backdrop-blur-sm" />
      </div>

      <div className="fixed top-20 left-10 w-20 h-20 rounded-full bg-peach/30 blur-2xl animate-float" />
      <div className="fixed top-40 right-20 w-32 h-32 rounded-full bg-lavender/25 blur-3xl animate-float" style={{ animationDelay: "2s" }} />
      <div className="fixed bottom-20 left-1/3 w-24 h-24 rounded-full bg-mint/30 blur-2xl animate-float" style={{ animationDelay: "4s" }} />

      <div className="relative z-10 w-full max-w-[90rem] mx-auto px-4 sm:px-6 py-8 space-y-5">
        <motion.div
          initial={{ y: -30, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          transition={{ duration: 0.6 }}
          className="flex items-end justify-between gap-4 flex-wrap"
        >
          <div className="flex items-center gap-3">
            <div>
              <h1 className="text-3xl sm:text-4xl font-display font-bold text-foreground tracking-tight">
                🌸 Cozy File Finder
              </h1>
              <p className="text-muted-foreground mt-1 text-sm">
                A warm little corner to find all your files
              </p>
            </div>
            {scanPhaseStatus !== "" ? (
              <div className="scan-phase-indicator">
                <span className="scan-phase-spinner" />
                <span className="scan-phase-label">{scanPhaseStatus}</span>
              </div>
            ) : null}
          </div>
          <IndexingControls
            indexedCount={filesCount}
            isIndexing={loading}
            onIndex={() => {
              debugLog("FileFindingView click Scan filesystem");
              onScan();
            }}
          />
        </motion.div>

        {error != null ? (
          <div className="bg-destructive/20 text-destructive-foreground px-4 py-3 rounded-xl">
            {error}
          </div>
        ) : null}

        {loading ? (
          <div className="glass rounded-2xl p-6 space-y-4">
            <div className="h-2 bg-muted rounded-full overflow-hidden">
              <div className="h-full w-1/3 bg-primary rounded-full animate-pulse" />
            </div>
            <p className="text-sm text-muted-foreground font-variant-numeric tabular-nums">
              {progress?.status != null
                ? progress.status
                : progress != null
                  ? `${progress.files_count.toLocaleString()} files scanned…`
                  : "Starting scan…"}
            </p>
            {progress?.current_path != null ? (
              <p className="text-xs text-muted-foreground truncate" title={progress.current_path}>
                {progress.current_path}
              </p>
            ) : null}
            {onCancelScan != null ? (
              <Button type="button" variant="secondary" size="sm" onClick={onCancelScan}>
                Cancel scan
              </Button>
            ) : null}
          </div>
        ) : null}

        {result !== null && !loading ? (
          <>
            {activeTab === "folders" ? (
              <FolderList folders={folderList} />
            ) : (
              <>
                <SearchBar
                  query={searchQuery}
                  onQueryChange={setSearchQuery}
                  useFuzzySearch={useFuzzySearch}
                  onFuzzySearchChange={setUseFuzzySearch}
                  extensionFilter={searchExtensions}
                  onExtensionFilterChange={setSearchExtensions}
                  disabled={!canSearch}
                />
                <FilterBar
                  activeCategory={searchCategory}
                  onCategoryChange={setSearchCategory}
                  disabled={!canSearch}
                />
                {searchError != null ? (
                  <div className="bg-destructive/20 text-destructive-foreground px-4 py-3 rounded-xl">
                    {searchError}
                  </div>
                ) : null}
                <FileTable
                  files={searchResults}
                  query={searchQuery}
                  searchLoading={searchLoading}
                  hasMore={canLoadMore}
                  loadingMore={searchLoadingMore}
                  onLoadMore={handleLoadMore}
                />
              </>
            )}
          </>
        ) : loading ? null : (
          <div className="glass-strong rounded-2xl p-12 text-center text-muted-foreground">
            <p className="text-lg font-display mb-2">Click &quot;Scan filesystem&quot; to start.</p>
            <p className="text-sm">Index your files to search and browse.</p>
          </div>
        )}
      </div>
    </div>
  );

  return view;
};
