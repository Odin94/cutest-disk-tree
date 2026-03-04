import type { ScanResult } from "../types";
import type { DiskTreeNode } from "../utils/diskTree";
import { humanSize, basename } from "../utils";
import { getDirectChildren } from "../utils/diskTree";
import { debugLog } from "../api";
import React from "react";

const SIDEBAR_LIST_LIMIT = 20;

type DiskUsageSidebarProps = {
  scan: ScanResult | null;
  tree: DiskTreeNode | null;
  hoveredPath: string | null;
};

const treeToPathMap = (node: DiskTreeNode): Map<string, DiskTreeNode> => {
  const map = new Map<string, DiskTreeNode>();
  const walk = (n: DiskTreeNode) => {
    map.set(n.path, n);
    if (n.children != null) {
      for (const c of n.children) walk(c);
    }
  };
  walk(node);
  return map;
};

const countTreeNodes = (node: DiskTreeNode): number => {
  let n = 1;
  if (node.children != null) {
    for (const c of node.children) n += countTreeNodes(c);
  }
  return n;
};

const childrenFromTree = (
  pathMap: Map<string, DiskTreeNode>,
  parentPath: string,
  limit: number
): { name: string; size: number; isFolder: boolean; isAggregated?: boolean }[] => {
  let node = pathMap.get(parentPath);
  if (node == null && parentPath.endsWith("_other")) {
    const withDouble = parentPath.replace(/_other$/, "__other");
    node = pathMap.get(withDouble);
  }
  const raw = node?.children ?? [];
  const display = raw.slice(0, limit);
  const rest = raw.slice(limit);
  const result: { name: string; size: number; isFolder: boolean; isAggregated?: boolean }[] = display.map((c) => ({
    name: c.name,
    size: c.size,
    isFolder: c.children != null && c.children.length > 0,
  }));
  if (rest.length > 0) {
    const aggregatedSize = rest.reduce((sum, c) => sum + c.size, 0);
    result.push({
      name: "smaller objects…",
      size: aggregatedSize,
      isFolder: false,
      isAggregated: true,
    });
  }
  return result;
};

const getChildrenForSidebarFallback = (
  scan: ScanResult,
  parentPath: string
): { name: string; size: number; isFolder: boolean; isAggregated?: boolean }[] => {
  const all = getDirectChildren(scan, parentPath, 100);
  const display = all.slice(0, SIDEBAR_LIST_LIMIT);
  const rest = all.slice(SIDEBAR_LIST_LIMIT);
  const result: { name: string; size: number; isFolder: boolean; isAggregated?: boolean }[] = display.map((c) => ({
    name: c.name,
    size: c.size,
    isFolder: c.isFolder,
  }));
  if (rest.length > 0) {
    const aggregatedSize = rest.reduce((sum, c) => sum + c.size, 0);
    result.push({
      name: "smaller objects…",
      size: aggregatedSize,
      isFolder: false,
      isAggregated: true,
    });
  }
  return result;
};

export const DiskUsageSidebar = ({
  scan,
  tree,
  hoveredPath,
}: DiskUsageSidebarProps) => {
  if (scan === null) {
    return (
      <aside className="disk-usage-sidebar">
        <p className="disk-usage-sidebar-hint">
          Look up sidebar for details.
        </p>
      </aside>
    );
  }

  const primaryRoot = scan.roots.length > 0 ? scan.roots[0] : "";
  const rootSize = primaryRoot.length > 0 ? (scan.folder_sizes[primaryRoot] ?? 0) : 0;
  const displayPath = hoveredPath ?? primaryRoot;
  const displayName = basename(displayPath);
  const displaySize = scan.folder_sizes[displayPath] ?? rootSize;

  const renderT0 = performance.now();

  const pathMap = React.useMemo(() => {
    const t0 = performance.now();
    const map = tree != null ? treeToPathMap(tree) : null;
    const ms = (performance.now() - t0).toFixed(2);
    const nodeCount = tree != null ? countTreeNodes(tree) : 0;
    debugLog(`sidebar_prof pathMap useMemo ran nodeCount=${nodeCount} ms=${ms}`);
    return map;
  }, [tree]);

  const children = React.useMemo(() => {
    const t0 = performance.now();
    const result =
      pathMap != null
        ? childrenFromTree(pathMap, displayPath, SIDEBAR_LIST_LIMIT)
        : getChildrenForSidebarFallback(scan, displayPath);
    const ms = (performance.now() - t0).toFixed(2);
    const source = pathMap != null ? "tree" : "fallback";
    debugLog(`sidebar_prof children useMemo source=${source} displayPath=${displayPath.slice(-50)} count=${result.length} ms=${ms}`);
    return result;
  }, [pathMap, displayPath, scan]);

  const renderMs = performance.now() - renderT0;
  if (renderMs > 10) {
    debugLog(
      `sidebar_prof sidebar render slow t=${performance.now().toFixed(1)} totalMs=${renderMs.toFixed(
        2
      )} displayPath=${displayPath.slice(-50)}`
    );
  }

  return (
    <aside className="disk-usage-sidebar">
      <div className="disk-usage-sidebar-head">
        <span className="disk-usage-sidebar-title" title={displayPath}>
          {displayName}
        </span>
        <span className="disk-usage-sidebar-size">{humanSize(displaySize)}</span>
      </div>
      {displayPath !== primaryRoot ? (
        <p className="disk-usage-sidebar-path" title={displayPath}>
          {displayPath}
        </p>
      ) : null}
      <ul className="disk-usage-sidebar-list">
        {children.map((child, idx) => (
          <li key={child.isAggregated ? "agg" : `${displayPath}-${idx}`} className="disk-usage-sidebar-item">
            <span
              className={
                child.isAggregated
                  ? "disk-usage-sidebar-dot disk-usage-sidebar-dot-muted"
                  : "disk-usage-sidebar-dot"
              }
            />
            <span
              className={
                child.isAggregated
                  ? "disk-usage-sidebar-name disk-usage-sidebar-name-muted"
                  : "disk-usage-sidebar-name"
              }
              title={child.name}
            >
              {child.name}
            </span>
            <span className="disk-usage-sidebar-item-size">
              {humanSize(child.size)}
            </span>
          </li>
        ))}
      </ul>
      {children.length === 0 ? (
        <p className="disk-usage-sidebar-hint">
          Look up sidebar for details.
        </p>
      ) : null}
      <p className="disk-usage-sidebar-footer-hint">
        Look up sidebar for details.
      </p>
    </aside>
  );
};
