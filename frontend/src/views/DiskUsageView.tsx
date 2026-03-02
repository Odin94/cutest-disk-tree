import { useState, useEffect, useRef, useCallback } from "react";
import {
  listCachedRoots,
  loadCachedScan,
  scanDirectory,
  pickDirectory,
  onScanProgress,
  debugLog,
  buildDiskTreeCached,
  listCachedTreeDepths,
} from "../api";
import type { ScanResult, ScanProgress } from "../types";
import type { DiskTreeNode } from "../utils/diskTree";
import { progressTopLevelFolder } from "../utils";
import { SunburstChart } from "../components/SunburstChart";
import { DiskUsageSidebar } from "../components/DiskUsageSidebar";
import { Button } from "../components/ui/button";

type DiskUsageViewProps = {
  onScanStart?: () => void;
  onScanDone?: () => void;
};

export const DiskUsageView = ({
  onScanStart,
  onScanDone,
}: DiskUsageViewProps) => {
  const [cachedRoots, setCachedRoots] = useState<string[]>([]);
  const [scans, setScans] = useState<Record<string, ScanResult>>({});
  const [selectedRoot, setSelectedRoot] = useState<string | null>(null);
  const [hoveredPath, setHoveredPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [scanProgress, setScanProgress] = useState<ScanProgress | null>(null);
  const [loadProgress, setLoadProgress] = useState<{
    current: number;
    total: number;
  } | null>(null);
  const [tree, setTree] = useState<DiskTreeNode | null>(null);
  const [treeLoading, setTreeLoading] = useState(false);
  const [scanUpdateCount, setScanUpdateCount] = useState(0);
  const treeRootRef = useRef<string | null>(null);
  const cancelLoadRef = useRef(false);
  const scanCancelRef = useRef(false);
  const scanProgressLogTimeRef = useRef(0);
  const PROGRESS_LOG_INTERVAL_MS = 2000;
  const TREE_INITIAL_DEPTH = 2;
  const TREE_AUTO_MAX_DEPTH = 3;
  const TREE_MAX_DEPTH = 4;
  const TREE_MAX_CHILDREN = 8;

  useEffect(() => {
    debugLog(`DiskUsageView loading=${loading} scanning=${scanning}`);
  }, [loading, scanning]);

  useEffect(() => {
    cancelLoadRef.current = false;
    let cancelled = false;
    debugLog("DiskUsageView initial load started");
    const load = async () => {
      setLoading(true);
      setLoadProgress(null);
      try {
        const roots = await listCachedRoots();
        debugLog(`DiskUsageView list_cached_roots returned count=${roots.length}`);
        if (cancelled || cancelLoadRef.current) {
          debugLog("DiskUsageView initial load cancelled (after list_cached_roots)");
          return;
        }
        setCachedRoots(roots);
        if (roots.length > 0 && selectedRoot === null) {
          setSelectedRoot(roots[0]);
        }
        if (roots.length > 0) {
          setLoadProgress({
            current: 0,
            total: roots.length,
          });
        }
        const next: Record<string, ScanResult> = {};
        const rootsToLoad = roots;
        let completed = 0;
        const results = await Promise.all(
          rootsToLoad.map(async (root) => {
            const scan = await loadCachedScan(root);
            completed += 1;
            if (!cancelled && !cancelLoadRef.current) {
              setLoadProgress({ current: completed, total: rootsToLoad.length });
            }
            return { root, scan };
          })
        );
        if (cancelled || cancelLoadRef.current) {
          debugLog("DiskUsageView initial load cancelled (after parallel load)");
          return;
        }
        for (const { root, scan } of results) {
          if (scan != null) {
            next[root] = scan;
          }
        }
        if (!cancelled && !cancelLoadRef.current) {
          setScans(next);
          setScanUpdateCount((c) => c + 1);
          debugLog("DiskUsageView initial load done");
        }
      } catch (e) {
        debugLog(
          `DiskUsageView initial load error: ${e instanceof Error ? e.message : String(e)}`
        );
        if (!cancelled && !cancelLoadRef.current) {
          setLoading(false);
        }
      } finally {
        if (!cancelled && !cancelLoadRef.current) {
          debugLog("DiskUsageView setLoading(false)");
          setLoading(false);
        }
      }
    };
    load();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (cachedRoots.length > 0 && selectedRoot === null) {
      setSelectedRoot(cachedRoots[0]);
    }
  }, [cachedRoots, selectedRoot]);

  useEffect(() => {
    const root = selectedRoot;
    if (root === null || loading) {
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
            listCachedTreeDepths(root, TREE_MAX_CHILDREN)
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
  }, [selectedRoot, scanUpdateCount, loading]);

  const startScanForPath = async (path: string) => {
    if (path.length === 0) {
      return;
    }
    debugLog(`DiskUsageView startScanForPath path=${path}`);
    onScanStart?.();
    setScanning(true);
    setScanProgress(null);
    scanCancelRef.current = false;
    scanProgressLogTimeRef.current = 0;
    try {
      const unlisten = await onScanProgress((progress) => {
        setScanProgress(progress);
        const now = Date.now();
        if (now - scanProgressLogTimeRef.current >= PROGRESS_LOG_INTERVAL_MS) {
          scanProgressLogTimeRef.current = now;
          const top = progress.current_path ?? "";
          debugLog(`DiskUsageView scan progress files=${progress.files_count} path=${top.slice(-50)}`);
        }
      });
      const data = await scanDirectory(path);
      unlisten();
      if (scanCancelRef.current) {
        debugLog("DiskUsageView startScanForPath completed but was cancelled");
        return;
      }
      debugLog(`DiskUsageView startScanForPath done path=${path} files=${data.files.length}`);
      setScans((prev) => ({ ...prev, [path]: data }));
      setScanUpdateCount((c) => c + 1);
      if (!cachedRoots.includes(path)) {
        setCachedRoots((prev) => [...prev, path]);
      }
      setSelectedRoot(path);
      onScanDone?.();
    } catch (e) {
      debugLog(
        `DiskUsageView startScanForPath error: ${e instanceof Error ? e.message : String(e)}`
      );
    } finally {
      setScanProgress(null);
      setScanning(false);
    }
  };

  const cancelLoad = () => {
    debugLog("DiskUsageView cancelLoad");
    cancelLoadRef.current = true;
  };

  const cancelScan = () => {
    debugLog("DiskUsageView cancelScan");
    scanCancelRef.current = true;
  };

  const runScan = async () => {
    debugLog("DiskUsageView runScan pickDirectory opening");
    const path = await pickDirectory();
    if (path === null) {
      debugLog("DiskUsageView runScan cancelled (no path)");
      return;
    }
    await startScanForPath(path);
  };

  const scanSelectedRoot = async () => {
    if (selectedRoot === null) {
      return;
    }
    debugLog(`DiskUsageView scanSelectedRoot root=${selectedRoot}`);
    await startScanForPath(selectedRoot);
  };

  const scan = selectedRoot != null ? scans[selectedRoot] ?? null : null;
  const totalSize = scan != null ? (scan.folder_sizes[scan.root] ?? 0) : 0;

  const handleChartHover = useCallback((path: string | null) => {
    const t0 = performance.now();
    debugLog(`sidebar_prof view onHover path=${path ?? "null"} t=${t0.toFixed(1)}`);
    setHoveredPath(path);
  }, []);

  if (loading) {
    const loadDetail =
      loadProgress != null && loadProgress.total > 0
        ? ` ${loadProgress.current}/${loadProgress.total}`
        : "";
    return (
      <div className="disk-usage-view disk-usage-view-loading">
        <div className="disk-usage-loading-overlay" role="status" aria-live="polite">
          <p className="disk-usage-loading-title">Restoring previous scans…</p>
          <p className="disk-usage-loading-detail">
            {loadProgress != null && loadProgress.total > 0
              ? `Loading cached scan${loadDetail}…`
              : "Listing cached roots…"}
          </p>
          <Button
            type="button"
            variant="secondary"
            onClick={() => {
              debugLog("DiskUsageView click Cancel (loading)");
              cancelLoad();
            }}
          >
            Cancel
          </Button>
        </div>
      </div>
    );
  }

  if (cachedRoots.length === 0) {
    return (
      <div className="disk-usage-view">
        <div className="disk-usage-empty">
          <p>No scanned folders yet.</p>
          <p>
            <Button
              type="button"
              onClick={() => {
                debugLog("DiskUsageView click Scan a folder");
                runScan();
              }}
              disabled={scanning}
            >
              {scanning ? "Scanning…" : "Scan a folder"}
            </Button>
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="disk-usage-view">
      <div className="disk-usage-main">
        <div className="disk-usage-chart-controls">
          {cachedRoots.length > 1 ? (
            <>
              <label className="disk-usage-root-label" htmlFor="disk-usage-root-select">
                Volume
              </label>
              <select
                id="disk-usage-root-select"
                className="disk-usage-root-select"
                value={selectedRoot ?? ""}
                onChange={(e) => {
                  const v = e.target.value || null;
                  debugLog(`DiskUsageView select Volume value=${v ?? "(empty)"}`);
                  setSelectedRoot(v);
                  setHoveredPath(null);
                }}
              >
                {cachedRoots.map((root) => (
                  <option key={root} value={root}>
                    {root}
                  </option>
                ))}
              </select>
            </>
          ) : null}
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => {
              debugLog("DiskUsageView click Scan another folder");
              runScan();
            }}
            disabled={scanning}
          >
            {scanning ? "Scanning…" : "Scan another folder"}
          </Button>
        </div>
        <div className="disk-usage-chart-area">
          {scan === null ? (
            <div className="disk-usage-loading">
              <p>No scan data stored for this folder yet.</p>
              <p>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() => {
                    debugLog("DiskUsageView click Scan this folder");
                    scanSelectedRoot();
                  }}
                  disabled={scanning}
                >
                  {scanning ? "Scanning…" : "Scan this folder"}
                </Button>
              </p>
            </div>
          ) : tree != null ? (
            <SunburstChart
              tree={tree}
              totalSize={totalSize}
              onHover={handleChartHover}
            />
          ) : treeLoading ? (
            <div className="disk-usage-loading">Building tree…</div>
          ) : (
            <div className="disk-usage-loading">No data for this folder.</div>
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
              {progressTopLevelFolder(
                selectedRoot,
                scanProgress.current_path ?? null
              ) != null ? (
                <p className="disk-usage-progress-text">
                  {`Top-level folder: ${progressTopLevelFolder(
                    selectedRoot,
                    scanProgress.current_path ?? null
                  )}`}
                </p>
              ) : null}
              {scanProgress.current_path != null ? (
                <p
                  className="disk-usage-progress-path"
                  title={scanProgress.current_path}
                >
                  {scanProgress.current_path}
                </p>
              ) : null}
              <Button
                type="button"
                variant="secondary"
                size="sm"
                className="disk-usage-progress-cancel"
                onClick={() => {
                  debugLog("DiskUsageView click Cancel scan");
                  cancelScan();
                }}
              >
                Cancel scan
              </Button>
            </div>
          ) : null}
        </div>
      </div>
      <DiskUsageSidebar scan={scan} tree={tree} hoveredPath={hoveredPath} />
    </div>
  );
};
