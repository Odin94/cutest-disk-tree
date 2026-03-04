import { useState, useEffect, useRef, useCallback } from "react";
import {
  debugLog,
  buildDiskTreeCached,
  listCachedTreeDepths,
} from "../api";
import type { ScanResult, ScanProgress } from "../types";
import type { DiskTreeNode } from "../utils/diskTree";
import { SunburstChart } from "../components/SunburstChart";
import { DiskUsageSidebar } from "../components/DiskUsageSidebar";
import { Button } from "../components/ui/button";

type DiskUsageViewProps = {
  result?: ScanResult | null;
  onScan?: () => void;
  scanning?: boolean;
  scanProgress?: ScanProgress | null;
};

export const DiskUsageView = ({
  result: externalResult,
  onScan,
  scanning = false,
  scanProgress = null,
}: DiskUsageViewProps) => {
  const [hoveredPath, setHoveredPath] = useState<string | null>(null);
  const [tree, setTree] = useState<DiskTreeNode | null>(null);
  const [treeLoading, setTreeLoading] = useState(false);
  const treeRootRef = useRef<string | null>(null);
  const TREE_INITIAL_DEPTH = 2;
  const TREE_AUTO_MAX_DEPTH = 3;
  const TREE_MAX_DEPTH = 4;
  const TREE_MAX_CHILDREN = 8;

  const scan = externalResult ?? null;
  const primaryRoot = scan !== null && scan.roots.length > 0 ? scan.roots[0] : null;
  const totalSize = scan !== null && primaryRoot !== null
    ? (scan.folder_sizes[primaryRoot] ?? 0)
    : 0;

  useEffect(() => {
    const root = primaryRoot;
    if (root === null) {
      setTree(null);
      setTreeLoading(false);
      treeRootRef.current = null;
      return;
    }
    let cancelled = false;
    treeRootRef.current = root;
    setTreeLoading(true);
    setTree(null);

    const loadNextDepth = (depth: number) => {
      if (cancelled || treeRootRef.current !== root) return;
      debugLog(`DiskUsageView building tree for root=${root} depth=${depth}`);
      buildDiskTreeCached(root, TREE_MAX_CHILDREN, depth)
        .then((diskTree) => {
          if (cancelled || treeRootRef.current !== root) return;
          debugLog(`DiskUsageView tree built depth=${depth}`);
          setTree(diskTree ?? null);
          if (depth === TREE_INITIAL_DEPTH) {
            setTreeLoading(false);
          }
          if (depth < TREE_AUTO_MAX_DEPTH) {
            loadNextDepth(depth + 1);
          } else if (depth === TREE_AUTO_MAX_DEPTH) {
            listCachedTreeDepths(TREE_MAX_CHILDREN)
              .then((cachedDepths) => {
                if (cancelled || treeRootRef.current !== root) return;
                const deeper = cachedDepths.filter(
                  (d) => d > TREE_AUTO_MAX_DEPTH && d <= TREE_MAX_DEPTH
                );
                deeper.sort((a, b) => a - b);
                const loadCachedDepth = (idx: number) => {
                  if (cancelled || treeRootRef.current !== root || idx >= deeper.length) return;
                  const d = deeper[idx];
                  debugLog(`DiskUsageView building tree for root=${root} depth=${d} (pre-cached)`);
                  buildDiskTreeCached(root, TREE_MAX_CHILDREN, d)
                    .then((diskTree) => {
                      if (cancelled || treeRootRef.current !== root) return;
                      debugLog(`DiskUsageView tree built depth=${d}`);
                      setTree(diskTree ?? null);
                      loadCachedDepth(idx + 1);
                    })
                    .catch(() => {});
                };
                loadCachedDepth(0);
              })
              .catch(() => {});
          }
        })
        .catch((e) => {
          if (!cancelled && treeRootRef.current === root) {
            debugLog(
              `DiskUsageView build tree error: ${e instanceof Error ? e.message : String(e)}`
            );
            setTreeLoading(false);
          }
        });
    };

    loadNextDepth(TREE_INITIAL_DEPTH);

    return () => {
      cancelled = true;
    };
  }, [primaryRoot]);

  const handleChartHover = useCallback((path: string | null) => {
    setHoveredPath(path);
  }, []);

  if (scan === null) {
    return (
      <div className="disk-usage-view">
        <div className="disk-usage-empty">
          <p>No scan data yet. Run a scan from the File finding tab, or wait for a cached scan to load.</p>
          {onScan != null ? (
            <p>
              <Button
                type="button"
                onClick={() => {
                  debugLog("DiskUsageView click Scan");
                  onScan();
                }}
                disabled={scanning}
              >
                {scanning ? "Scanning…" : "Scan filesystem"}
              </Button>
            </p>
          ) : null}
        </div>
      </div>
    );
  }

  return (
    <div className="disk-usage-view">
      <div className="disk-usage-main">
        <div className="disk-usage-chart-controls">
          {onScan != null ? (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => {
                debugLog("DiskUsageView click Rescan");
                onScan();
              }}
              disabled={scanning}
            >
              {scanning ? "Scanning…" : "Rescan filesystem"}
            </Button>
          ) : null}
        </div>
        <div className="disk-usage-chart-area">
          {tree != null ? (
            <SunburstChart
              tree={tree}
              totalSize={totalSize}
              onHover={handleChartHover}
            />
          ) : treeLoading ? (
            <div className="disk-usage-loading">Building tree…</div>
          ) : (
            <div className="disk-usage-loading">No data for the filesystem.</div>
          )}
          {scanning && scanProgress != null ? (
            <div className="disk-usage-progress-overlay">
              <div className="disk-usage-progress-bar" role="status">
                <div className="disk-usage-progress-bar-inner" />
              </div>
              <p className="disk-usage-progress-text">
                {scanProgress.status != null
                  ? scanProgress.status
                  : `${scanProgress.files_count.toLocaleString()} files scanned…`}
              </p>
              <p className="disk-usage-progress-text">
                {`${scanProgress.files_count.toLocaleString()} files scanned`}
              </p>
              {scanProgress.current_path != null ? (
                <p
                  className="disk-usage-progress-path"
                  title={scanProgress.current_path}
                >
                  {scanProgress.current_path}
                </p>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
      <DiskUsageSidebar scan={scan} tree={tree} hoveredPath={hoveredPath} />
    </div>
  );
};
