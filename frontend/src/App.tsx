import { useState, useRef, useEffect } from "react";
import { scanDirectory, pickDirectory, onScanProgress, loadCachedScan, debugLog, onScanPhaseStatus, onScanFolderSizesReady } from "./api";
import type { ScanResult, ScanProgress, FolderSizesReady } from "./types";
import "./App.css";
import { DiskUsageView } from "./views/DiskUsageView";
import { FileFindingView } from "./views/FileFindingView";

type CategoryId = "disk" | "find";

const CheckForUpdatesButton = () => {
  const [checking, setChecking] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const check = async () => {
    debugLog("App click Check for updates");
    setChecking(true);
    setMessage(null);
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();
      if (update != null) {
        setMessage(`Update ${update.version} available. Downloading…`);
        const { relaunch } = await import("@tauri-apps/plugin-process");
        await update.downloadAndInstall();
        setMessage("Update installed. Restarting…");
        await relaunch();
      } else {
        setMessage("No updates available.");
      }
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setChecking(false);
    }
  };

  return (
    <span className="updater-wrap">
      <button
        type="button"
        className="secondary"
        onClick={check}
        disabled={checking}
      >
        {checking ? "Checking…" : "Check for updates"}
      </button>
      {message != null ? (
        <span className="updater-message">{message}</span>
      ) : null}
    </span>
  );
};

const App = () => {
  const [category, setCategory] = useState<CategoryId>("disk");
  const [result, setResult] = useState<ScanResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [scanRootPath, setScanRootPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scanPhaseStatus, setScanPhaseStatus] = useState<string>("");
  const unlistenRef = useRef<(() => void) | null>(null);
  const phaseStatusUnlistenRef = useRef<(() => void) | null>(null);
  const scanCancelRef = useRef(false);
  const progressLogTimeRef = useRef(0);
  const PROGRESS_LOG_INTERVAL_MS = 2000;
  // Holds the most recently received folder-sizes event payload. Phase 2 can
  // emit this before scanDirectory's invoke promise resolves, so we stash it
  // here and apply it right after setResult(data) to avoid the race.
  const latestFolderSizesRef = useRef<FolderSizesReady | null>(null);

  const runScan = async () => {
    debugLog("App runScan pickDirectory opened");
    const path = await pickDirectory();
    if (path === null) {
      debugLog("App runScan cancelled (no path)");
      return;
    }
    debugLog(`App runScan initiated path=${path}`);
    latestFolderSizesRef.current = null;
    setScanRootPath(path);
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
      const data = await scanDirectory(path);
      if (scanCancelRef.current) {
        debugLog("App runScan completed but was cancelled, ignoring result");
        return;
      }
      debugLog(`App runScan done path=${path} files=${data.files.length}`);
      setResult(data);
      // If the folder-sizes event already arrived before this point, apply it
      // now on top of the just-set result (React batches these two updates).
      const latestSizes = latestFolderSizesRef.current as FolderSizesReady | null;
      if (latestSizes !== null && latestSizes.root === path) {
        const folderSizes = latestSizes.folder_sizes;
        setResult((prev) =>
          prev !== null ? { ...prev, folder_sizes: folderSizes } : prev
        );
      }
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
      setLoading(false);
      setProgress(null);
      setScanRootPath(null);
    }
  };

  const cancelScan = () => {
    debugLog("App cancelScan");
    scanCancelRef.current = true;
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }
    setLoading(false);
    setProgress(null);
    setScanRootPath(null);
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
      // Always stash the payload so runScan can pick it up even if it arrives
      // before setResult(data) runs.
      latestFolderSizesRef.current = payload;
      setResult((prev) =>
        prev !== null && prev.root === payload.root
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

  const handleSelectCachedRoot = async (root: string) => {
    debugLog(`App handleSelectCachedRoot root=${root}`);
    try {
      const scan = await loadCachedScan(root);
      if (scan != null) setResult(scan);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError("Failed to load cached scan.");
      debugLog(`App handleSelectCachedRoot error: ${msg}`);
    }
  };

  return (
    <div
      className={`app ${category === "disk" ? "app-dashboard-layout" : ""}`}
    >
      <header className="header">
        <h1>Cutest Disk Tree</h1>
        <div className="header-actions">
          {scanPhaseStatus !== "" ? (
            <div className="scan-phase-indicator">
              <span className="scan-phase-spinner" />
              <span className="scan-phase-label">{scanPhaseStatus}</span>
            </div>
          ) : null}
          <CheckForUpdatesButton />
        </div>
      </header>

      <nav className="nav-categories">
        <button
          type="button"
          className={category === "disk" ? "active" : ""}
          onClick={() => {
            debugLog("App setCategory disk");
            setCategory("disk");
          }}
        >
          Disk usage
        </button>
        <button
          type="button"
          className={category === "find" ? "active" : ""}
          onClick={() => {
            debugLog("App setCategory find");
            setCategory("find");
          }}
        >
          File finding
        </button>
      </nav>

      <div className="views">
        <div
          className="view view-disk-usage"
          style={{ display: category === "disk" ? "block" : "none" }}
        >
          <DiskUsageView externalScanRoot={result ? result.root : null} />
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
            scanRootPath={scanRootPath}
            onScan={runScan}
            onCancelScan={cancelScan}
            onSelectCachedRoot={handleSelectCachedRoot}
          />
        </div>
      </div>
    </div>
  );
};

export default App;
