import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { ScanResult, ScanProgress, FileEntry } from "./types";

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

export const findFiles = (
  root: string,
  query: string,
  extensions: string
): Promise<FileEntry[]> =>
  invoke("find_files", {
    root,
    query,
    extensions: extensions.trim().length > 0 ? extensions : null,
  });

export const pickDirectory = async (): Promise<string | null> => {
  const selected = await open({
    directory: true,
    multiple: false,
  });
  return selected ?? null;
};
