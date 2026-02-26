import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { ScanResult } from "./types";

export const scanDirectory = (path: string): Promise<ScanResult> =>
  invoke("scan_directory", { path });

export const pickDirectory = async (): Promise<string | null> => {
  const selected = await open({
    directory: true,
    multiple: false,
  });
  return selected ?? null;
};
