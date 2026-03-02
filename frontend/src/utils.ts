export const humanSize = (bytes: number): string => {
  const KB = 1024;
  const MB = KB * 1024;
  const GB = MB * 1024;
  const TB = GB * 1024;
  if (bytes >= TB) return `${(bytes / TB).toFixed(1)} TB`;
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`;
  if (bytes >= KB) return `${(bytes / KB).toFixed(1)} KB`;
  return `${bytes} B`;
};

export const basename = (path: string): string => {
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(sep);
  return parts[parts.length - 1] ?? path;
};

export const progressTopLevelFolder = (
  rootPath: string | null,
  currentPath: string | null | undefined
): string | null => {
  if (rootPath === null || rootPath.length === 0) {
    return null;
  }
  if (currentPath == null || currentPath.length === 0) {
    return basename(rootPath);
  }
  const sep = rootPath.includes("\\") ? "\\" : "/";
  const normalizedRoot = rootPath.endsWith(sep)
    ? rootPath.slice(0, -1)
    : rootPath;
  const normalizedCurrent = currentPath.replace(/[\\/]+/g, sep);
  let relative = normalizedCurrent;
  if (normalizedCurrent.startsWith(normalizedRoot + sep)) {
    relative = normalizedCurrent.slice(normalizedRoot.length + sep.length);
  }
  if (relative.length === 0) {
    return basename(rootPath);
  }
  const firstSepIndex = relative.indexOf(sep);
  if (firstSepIndex === -1) {
    return relative;
  }
  return relative.slice(0, firstSepIndex);
};

