export type FileKey = { dev: number; ino: number };

export type FileEntry = {
  path: string;
  size: number;
  file_key: FileKey;
};

export type ScanProgress = {
  files_count: number;
  current_path?: string;
  status?: string;
};

export type ScanResult = {
  root: string;
  files: FileEntry[];
  folder_sizes: Record<string, number>;
};

export type FileSearchResult = FileEntry;
