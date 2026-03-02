import type { ScanResult, FileEntry } from "../types";
import { basename } from "../utils";

export type DiskTreeNode = {
  path: string;
  name: string;
  size: number;
  children?: DiskTreeNode[];
};

type BuildOptions = {
  maxChildrenPerNode?: number;
  maxDepth?: number;
};

const getSeparator = (path: string): string => {
  return path.includes("\\") ? "\\" : "/";
};

const normalizePath = (path: string): string => {
  const trimmed = path.trim();
  if (trimmed.length === 0) {
    return trimmed;
  }
  const sep = getSeparator(trimmed);
  const parts = trimmed.split(/[\\/]+/).filter((segment) => segment.length > 0);
  if (parts.length === 0) {
    return trimmed;
  }
  return parts.join(sep);
};

const parentDir = (path: string): string => {
  const sep = getSeparator(path);
  const parts = path.split(sep);
  if (parts.length <= 1) {
    return "";
  }
  return parts.slice(0, -1).join(sep);
};

const directChildName = (parentPath: string, childPath: string): string | null => {
  const parentNorm = normalizePath(parentPath);
  const childNorm = normalizePath(childPath);

  if (parentNorm.length === 0) {
    return basename(childNorm);
  }

  if (parentNorm === childNorm) {
    return null;
  }

  const sep = getSeparator(childNorm);
  if (!childNorm.startsWith(parentNorm + sep)) {
    return null;
  }

  const rest = childNorm.slice(parentNorm.length + sep.length);
  if (rest.length === 0 || rest.includes(sep)) {
    return null;
  }
  return rest;
};

const collectFolderChildren = (
  parentPath: string,
  folderSizes: Record<string, number>
): DiskTreeNode[] => {
  const children: DiskTreeNode[] = [];
  for (const [folderPath, size] of Object.entries(folderSizes)) {
    const name = directChildName(parentPath, folderPath);
    if (name === null) {
      continue;
    }
    children.push({
      path: folderPath,
      name,
      size,
    });
  }
  return children;
};

const collectFileChildren = (parentPath: string, files: FileEntry[]): DiskTreeNode[] => {
  const children: DiskTreeNode[] = [];
  for (const file of files) {
    const normalized = normalizePath(file.path);
    const fileParent = parentDir(normalized);
    const normalizedParent = normalizePath(parentPath);
    if (normalizedParent.length === 0) {
      continue;
    }
    if (fileParent !== normalizedParent) {
      continue;
    }
    const name = basename(normalized);
    children.push({
      path: normalized,
      name,
      size: file.size,
    });
  }
  return children;
};

export const buildDiskTree = (
  scan: ScanResult,
  options?: BuildOptions
): DiskTreeNode | null => {
  const rootSize = scan.folder_sizes[scan.root];
  if (rootSize == null) {
    return null;
  }

  const maxChildrenPerNode = options?.maxChildrenPerNode ?? 20;
  const maxDepth = options?.maxDepth ?? 5;

  const buildNode = (path: string, depth: number): DiskTreeNode => {
    const size = scan.folder_sizes[path] ?? 0;
    const folderChildren = collectFolderChildren(path, scan.folder_sizes);
    const fileChildren = collectFileChildren(path, scan.files);

    const combined = [...folderChildren, ...fileChildren].sort(
      (a, b) => b.size - a.size
    );

    const limited = combined.slice(0, maxChildrenPerNode);

    if (depth >= maxDepth || limited.length === 0) {
      return {
        path,
        name: basename(path),
        size,
      };
    }

    const children: DiskTreeNode[] = limited.map((child) => {
      if (scan.folder_sizes[child.path] != null) {
        return buildNode(child.path, depth + 1);
      }
      return child;
    });

    return {
      path,
      name: basename(path),
      size,
      children,
    };
  };

  return buildNode(scan.root, 0);
};

export type DirectChild = {
  name: string;
  path: string;
  size: number;
  isFolder: boolean;
};

export const getDirectChildren = (
  scan: ScanResult,
  parentPath: string,
  limit = 20
): DirectChild[] => {
  const folderChildren = collectFolderChildren(
    parentPath,
    scan.folder_sizes
  ).map((n) => ({ name: n.name, path: n.path, size: n.size, isFolder: true }));
  const fileChildren = collectFileChildren(parentPath, scan.files).map((n) => ({
    name: n.name,
    path: n.path,
    size: n.size,
    isFolder: false,
  }));
  const combined = [...folderChildren, ...fileChildren].sort(
    (a, b) => b.size - a.size
  );
  return combined.slice(0, limit);
};

