import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ScanDirectoryResponse, ScanProgress, FileSearchResult, FolderSizesReady } from "./types";
import type { DiskTreeNode } from "./utils/diskTree";

export const scanDirectoryWithHelper = (): Promise<ScanDirectoryResponse> =>
  invoke("scan_directory_with_helper", {});

export const buildDiskTreeCached = (
  startPath: string,
  maxChildrenPerNode: number,
  maxDepth: number
): Promise<DiskTreeNode | null> =>
  invoke("build_disk_tree_cached", {
    startPath,
    maxChildrenPerNode,
    maxDepth,
  });

export const debugLog = (message: string): void => {
  invoke("debug_log", { message }).catch(() => {});
};

export const getDebugLogPath = (): Promise<string> =>
  invoke("get_debug_log_path", {});

export const debugLogStats = (message: string): void => {
  invoke("debug_log_stats", { message }).catch(() => {});
};

export const scanDirectory = (): Promise<ScanDirectoryResponse> => {
  const stack = new Error().stack ?? "(no stack)";
  console.error(`[scanDirectory] invoke called.\n${stack}`);
  debugLog(`scanDirectory invoked stack=${stack.split("\n").slice(0, 5).join(" | ")}`);
  return invoke("scan_directory", {});
};

export const onScanProgress = (callback: (progress: ScanProgress) => void) => {
  const unlisten = listen<ScanProgress>("scan-progress", (event) => {
    callback(event.payload);
  });
  return unlisten;
};

export const onScanPhaseStatus = (callback: (status: string) => void) => {
  const unlisten = listen<string>("scan-phase-status", (event) => {
    callback(event.payload);
  });
  return unlisten;
};

export const onScanFolderSizesReady = (
  callback: (payload: FolderSizesReady) => void
) => {
  const unlisten = listen<FolderSizesReady>("scan-folder-sizes-ready", (event) => {
    callback(event.payload);
  });
  return unlisten;
};

export type CachedScanSummary = {
  roots: string[];
  files_count: number;
  folders_count: number;
  folder_sizes: Record<string, number>;
};

export const loadCachedScan = (): Promise<CachedScanSummary | null> =>
  invoke("load_cached_scan", {});

export const listCachedTreeDepths = (
  maxChildrenPerNode: number
): Promise<number[]> =>
  invoke("list_cached_tree_depths", { maxChildren: maxChildrenPerNode });

export type FindFilesResponse = {
  items: FileSearchResult[];
  nextOffset: number | null;
};

export const findFiles = (
  query: string,
  extensions: string,
  category: string,
  useFuzzy: boolean,
  limit?: number,
  offset?: number
): Promise<FindFilesResponse> =>
  invoke("find_files", {
    query,
    extensions: extensions.trim().length > 0 ? extensions.trim() : null,
    category: category.trim() !== "" && category !== "all" ? category : null,
    limit: limit ?? 500,
    use_fuzzy: useFuzzy,
    offset: offset ?? 0,
  });
