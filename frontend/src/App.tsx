import { useState, useRef, useEffect } from "react";
import { Toaster } from "sonner";
import { scanDirectory, onScanProgress, loadCachedScan, debugLog, onScanPhaseStatus, onScanFolderSizesReady } from "./api";
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
  const unlistenRef = useRef<(() => void) | null>(null);
  const phaseStatusUnlistenRef = useRef<(() => void) | null>(null);
  const scanCancelRef = useRef(false);
  const scanInProgressRef = useRef(false);
  const progressLogTimeRef = useRef(0);
  const PROGRESS_LOG_INTERVAL_MS = 2000;
  const latestFolderSizesRef = useRef<FolderSizesReady | null>(null);

  const runScan = async () => {
    if (scanInProgressRef.current) {
      const stack = new Error().stack ?? "(no stack)";
      debugLog(`App runScan BLOCKED (already in progress) stack=${stack.split("\n").slice(0, 5).join(" | ")}`);
      console.error("[runScan] BLOCKED - scan already in progress", stack);
      return;
    }
    scanInProgressRef.current = true;
    debugLog("App runScan initiated");
    latestFolderSizesRef.current = null;
    setLoading(true);
    setProgress(null);
    setError(null);
    scanCancelRef.current = false;
    progressLogTimeRef.current = 0;
    try {
      debugLog("App runScan: setting up progress listener");
      unlistenRef.current = await onScanProgress((p) => {
        setProgress(p);
        const now = Date.now();
        if (now - progressLogTimeRef.current >= PROGRESS_LOG_INTERVAL_MS) {
          progressLogTimeRef.current = now;
          const top = p.current_path ?? "";
          debugLog(`App scan progress files=${p.files_count} current_path=${top.slice(-60)}`);
        }
      });
      debugLog("App runScan: calling scanDirectory");
      const summary: ScanDirectoryResponse = await scanDirectory();
      debugLog(`App runScan: scanDirectory returned files_count=${summary.files_count} folders_count=${summary.folders_count}`);
      if (scanCancelRef.current) {
        debugLog("App runScan completed but was cancelled, ignoring result");
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
      debugLog(`App runScan done files_count=${summary.files_count}`);
      setResult(scanResult);
      if (category !== "find") setCategory("find");
    } catch (e) {
      if (!scanCancelRef.current) {
        setError(e instanceof Error ? e.message : String(e));
        debugLog(`App runScan error ${e instanceof Error ? e.message : String(e)}`);
      }
    } finally {
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
      scanInProgressRef.current = false;
      setLoading(false);
      setProgress(null);
      debugLog("App runScan finally: cleanup done, scanInProgressRef=false");
    }
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

  useEffect(() => {
    debugLog("App mount: loading cached scan");
    loadCachedScan().then((summary) => {
      if (summary != null) {
        debugLog(`App cached scan loaded files_count=${summary.files_count} folders_count=${summary.folders_count}`);
        setResult({
          roots: summary.roots,
          files: [],
          folder_sizes: summary.folder_sizes,
          files_count: summary.files_count,
          folders_count: summary.folders_count,
        });
      } else {
        debugLog("App no cached scan found");
      }
    }).catch((e) => {
      debugLog(`App loadCachedScan error: ${e instanceof Error ? e.message : String(e)}`);
    });
  }, []);

  return (
    <div
      className={`app ${category === "disk" ? "app-dashboard-layout" : ""}`}
    >
      <Toaster />
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
        <div
          className="view view-disk-usage"
          style={{ display: category === "disk" ? "block" : "none" }}
        >
          <DiskUsageView
            result={result}
            onScan={runScan}
            scanning={loading}
            scanProgress={progress}
          />
        </div>
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
