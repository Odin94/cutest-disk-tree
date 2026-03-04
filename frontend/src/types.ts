export type FileKey = { dev: number; ino: number };

export type FileEntry = {
  path: string;
  size: number;
  file_key: FileKey;
  mtime?: number;
};

export type SearchItemKind = "file" | "folder";

export type ScanProgress = {
  files_count: number;
  current_path?: string;
  status?: string;
};

export type ScanResult = {
  roots: string[];
  files: FileEntry[];
  folder_sizes: Record<string, number>;
  files_count?: number;
  folders_count?: number;
};

export type ScanDirectoryResponse = {
  roots: string[];
  files_count: number;
  folders_count: number;
};

export type FolderSizesReady = {
  folder_sizes: Record<string, number>;
};

export type FileSearchResult = {
  kind: SearchItemKind;
  path: string;
  size: number;
  file_key?: FileKey;
};

