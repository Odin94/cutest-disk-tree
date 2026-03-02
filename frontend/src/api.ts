import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { ScanResult, ScanProgress, FileSearchResult } from "./types";
import type { DiskTreeNode } from "./utils/diskTree";

export const buildDiskTreeCached = (
  root: string,
  maxChildrenPerNode: number,
  maxDepth: number
): Promise<DiskTreeNode | null> =>
  invoke("build_disk_tree_cached", {
    root,
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

export const scanDirectory = (path: string): Promise<ScanResult> =>
  invoke("scan_directory", { path });

export const onScanProgress = (callback: (progress: ScanProgress) => void) => {
  const unlisten = listen<ScanProgress>("scan-progress", (event) => {
    callback(event.payload);
  });
  return unlisten;
};

export const listCachedRoots = (): Promise<string[]> =>
  invoke("list_cached_roots", {});

export const loadCachedScan = (root: string): Promise<ScanResult | null> =>
  invoke("load_cached_scan", { root });

export const listCachedTreeDepths = (
  root: string,
  maxChildrenPerNode: number
): Promise<number[]> =>
  invoke("list_cached_tree_depths", { root, maxChildren: maxChildrenPerNode });

export const findFiles = (
  root: string,
  query: string,
  extensions: string,
  useFuzzy: boolean,
  limit?: number
): Promise<FileSearchResult[]> =>
  invoke("find_files", {
    root,
    query,
    extensions: extensions.trim().length > 0 ? extensions : null,
    limit: limit ?? 500,
    use_fuzzy: useFuzzy,
  });

export const pickDirectory = async (): Promise<string | null> => {
  const selected = await open({
    directory: true,
    multiple: false,
  });
  return selected ?? null;
};
