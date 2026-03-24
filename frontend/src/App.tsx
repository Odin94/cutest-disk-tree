import { useState, useRef, useEffect } from "react";
import { Toaster } from "sonner";
import { scanDirectory, scanDirectoryWithHelper, onScanProgress, onScanComplete, getScanStatus, loadCachedScan, debugLog, onScanPhaseStatus, onScanFolderSizesReady } from "./api";
import type { ScanResult, ScanProgress, FolderSizesReady, ScanDirectoryResponse } from "./types";
import "./App.css";
import { DiskUsageView } from "./views/DiskUsageView";
import { FileFindingView, type TabId } from "./views/FileFindingView";
import { TitleBar } from "./components/TitleBar";

type CategoryId = "disk" | "find";

const App = () => {
  const [category, setCategory] = useState<CategoryId>("find");
  const [activeTab, setActiveTab] = useState<TabId>("find");
  const [result, setResult] = useState<ScanResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scanPhaseStatus, setScanPhaseStatus] = useState<string>("");
  const [showElevationDialog, setShowElevationDialog] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);
  const phaseStatusUnlistenRef = useRef<(() => void) | null>(null);
  const scanCancelRef = useRef(false);
  const scanInProgressRef = useRef(false);
  const observingScanRef = useRef(false);
  const progressLogTimeRef = useRef(0);
  const PROGRESS_LOG_INTERVAL_MS = 2000;
  const latestFolderSizesRef = useRef<FolderSizesReady | null>(null);

  const executeScan = async (
    invoker: () => Promise<ScanDirectoryResponse>,
    label: string
  ) => {
    if (scanInProgressRef.current) {
      const stack = new Error().stack ?? "(no stack)";
      debugLog(`App ${label} BLOCKED (already in progress) stack=${stack.split("\n").slice(0, 5).join(" | ")}`);
      return;
    }
    scanInProgressRef.current = true;
    debugLog(`App ${label} initiated`);
    latestFolderSizesRef.current = null;
    setLoading(true);
    setProgress(null);
    setError(null);
    scanCancelRef.current = false;
    progressLogTimeRef.current = 0;
    try {
      unlistenRef.current = await onScanProgress((p) => {
        setProgress(p);
        const now = Date.now();
        if (now - progressLogTimeRef.current >= PROGRESS_LOG_INTERVAL_MS) {
          progressLogTimeRef.current = now;
          const top = p.current_path ?? "";
          debugLog(`App scan progress files=${p.files_count} current_path=${top.slice(-60)}`);
        }
      });
      const summary: ScanDirectoryResponse = await invoker();
      debugLog(`App ${label}: returned files_count=${summary.files_count} folders_count=${summary.folders_count}`);
      if (scanCancelRef.current) {
        debugLog(`App ${label} completed but was cancelled, ignoring result`);
        return;
      }
      const scanResult: ScanResult = {
        roots: summary.roots,
        files: [],
        folder_sizes: {},
        files_count: summary.files_count,
        folders_count: summary.folders_count,
      };
      const latestSizes = latestFolderSizesRef.current as FolderSizesReady | null;
      if (latestSizes !== null) {
        scanResult.folder_sizes = latestSizes.folder_sizes;
      }
      debugLog(`App ${label} done files_count=${summary.files_count}`);
      setResult(scanResult);
      if (category !== "find") setCategory("find");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg === "A scan is already in progress") {
        debugLog(`App ${label} scan already in progress, entering observer mode`);
        observingScanRef.current = true;
        // Stay in loading state — scan-complete event will finish cleanup
        return;
      }
      if (!scanCancelRef.current) {
        setError(msg);
        debugLog(`App ${label} error ${msg}`);
      }
    } finally {
      if (observingScanRef.current) {
        // Observer mode: scan-complete handler is responsible for cleanup
        debugLog(`App ${label} finally: observer mode, skipping cleanup`);
        return;
      }
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
      scanInProgressRef.current = false;
      setLoading(false);
      setProgress(null);
      debugLog(`App ${label} finally: cleanup done`);
    }
  };

  const runScan = () => {
    if (scanInProgressRef.current) return;
    setShowElevationDialog(true);
  };

  const cancelScan = () => {
    debugLog("App cancelScan");
    scanCancelRef.current = true;
    scanInProgressRef.current = false;
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }
    setLoading(false);
    setProgress(null);
  };

  useEffect(() => {
    let isMounted = true;
    onScanPhaseStatus((status) => {
      if (!isMounted) return;
      setScanPhaseStatus(status);
    }).then((unlisten) => {
      if (!isMounted) {
        unlisten();
      } else {
        phaseStatusUnlistenRef.current = unlisten;
      }
    });
    return () => {
      isMounted = false;
      if (phaseStatusUnlistenRef.current) {
        phaseStatusUnlistenRef.current();
        phaseStatusUnlistenRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    let isMounted = true;
    let unlisten: (() => void) | null = null;
    onScanFolderSizesReady((payload) => {
      if (!isMounted) return;
      latestFolderSizesRef.current = payload;
      setResult((prev) =>
        prev !== null
          ? { ...prev, folder_sizes: payload.folder_sizes }
          : prev
      );
    }).then((fn) => {
      if (!isMounted) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      isMounted = false;
      if (unlisten !== null) unlisten();
    };
  }, []);

  // Listen for scan-complete to handle observer mode (scan started before/outside this session)
  useEffect(() => {
    let isMounted = true;
    onScanComplete((summary) => {
      if (!isMounted || !observingScanRef.current) return;
      debugLog(`App scan-complete observer: files_count=${summary.files_count}`);
      observingScanRef.current = false;
      scanInProgressRef.current = false;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
      setLoading(false);
      setProgress(null);
      setResult({
        roots: summary.roots,
        files: [],
        folder_sizes: latestFolderSizesRef.current?.folder_sizes ?? {},
        files_count: summary.files_count,
        folders_count: summary.folders_count,
      });
    }).then((unlisten) => {
      if (!isMounted) unlisten();
    });
    return () => { isMounted = false; };
  }, []);

  // On startup, check if a scan is already in progress and show loading UI
  useEffect(() => {
    const t = performance.now();
    getScanStatus().then((isScanning) => {
      debugLog(`App getScanStatus ms=${Math.round(performance.now() - t)} isScanning=${isScanning}`);
      if (isScanning) {
        observingScanRef.current = true;
        scanInProgressRef.current = true;
        setLoading(true);
        onScanProgress((p) => setProgress(p)).then((fn) => {
          unlistenRef.current = fn;
        });
      }
    }).catch((e) => {
      debugLog(`App getScanStatus error: ${e instanceof Error ? e.message : String(e)}`);
    });
  }, []);

  useEffect(() => {
    const t = performance.now();
    debugLog("App mount: loading cached scan");
    loadCachedScan().then((summary) => {
      const loadMs = Math.round(performance.now() - t);
      if (summary != null) {
        debugLog(`App cached scan loaded files_count=${summary.files_count} folders_count=${summary.folders_count} load_ms=${loadMs}`);
        setResult({
          roots: summary.roots,
          files: [],
          folder_sizes: summary.folder_sizes,
          files_count: summary.files_count,
          folders_count: summary.folders_count,
        });
      } else {
        debugLog(`App no cached scan found load_ms=${loadMs}`);
      }
    }).catch((e) => {
      debugLog(`App loadCachedScan error: ${e instanceof Error ? e.message : String(e)}`);
    });
  }, []);

  const handleChooseFastScan = () => {
    setShowElevationDialog(false);
    executeScan(scanDirectoryWithHelper, "runScanFast");
  };

  const handleChooseSlowScan = () => {
    setShowElevationDialog(false);
    executeScan(scanDirectory, "runScanSlow");
  };

  return (
    <div
      className={`app ${category === "disk" ? "app-dashboard-layout" : ""}`}
    >
      <Toaster />

      {showElevationDialog && (
        <div className="elevation-dialog-overlay" onClick={() => setShowElevationDialog(false)}>
          <div className="elevation-dialog" onClick={(e) => e.stopPropagation()}>
            <div className="elevation-dialog-header">
              <h2 className="elevation-dialog-title">Choose scan speed</h2>
              <button
                type="button"
                className="elevation-dialog-close"
                onClick={() => setShowElevationDialog(false)}
                aria-label="Close"
              >
                ✕
              </button>
            </div>
            <p className="elevation-dialog-body">
              Fast scanning reads the file system index directly (MFT) but
              requires administrator rights. Slow scanning walks directories
              without any special permissions.
            </p>
            <div className="elevation-dialog-actions">
              <button
                type="button"
                className="elevation-dialog-btn elevation-dialog-btn-fast"
                onClick={handleChooseFastScan}
              >
                Fast scan
                <span className="elevation-dialog-btn-sub">runs as admin · triggers UAC prompt</span>
              </button>
              <button
                type="button"
                className="elevation-dialog-btn elevation-dialog-btn-slow"
                onClick={handleChooseSlowScan}
              >
                Slow scan
                <span className="elevation-dialog-btn-sub">no admin needed</span>
              </button>
            </div>
          </div>
        </div>
      )}

      <TitleBar
        category={category}
        activeTab={activeTab}
        onNavigate={(cat, tab) => {
          debugLog(`App navigate category=${cat} tab=${tab ?? ""}`);
          setCategory(cat);
          if (tab != null) setActiveTab(tab);
        }}
      />

      <div className="views">
        {category === "disk" && (
          <div className="view view-disk-usage">
            <DiskUsageView
              result={result}
              onScan={runScan}
              scanning={loading}
              scanProgress={progress}
            />
          </div>
        )}
        <div
          className="view view-file-finding"
          style={{ display: category === "find" ? "block" : "none" }}
        >
          <FileFindingView
            result={result}
            loading={loading}
            error={error}
            progress={progress}
            scanPhaseStatus={scanPhaseStatus}
            onScan={runScan}
            onCancelScan={cancelScan}
            activeTab={activeTab}
            onTabChange={setActiveTab}
          />
        </div>
      </div>
    </div>
  );
};

export default App;
