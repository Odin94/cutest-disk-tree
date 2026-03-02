import { useState, useRef } from "react";
import { scanDirectory, pickDirectory, onScanProgress, loadCachedScan, debugLog } from "./api";
import type { ScanResult, ScanProgress } from "./types";
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
  const unlistenRef = useRef<(() => void) | null>(null);
  const scanCancelRef = useRef(false);
  const progressLogTimeRef = useRef(0);
  const PROGRESS_LOG_INTERVAL_MS = 2000;

  const runScan = async () => {
    debugLog("App runScan pickDirectory opened");
    const path = await pickDirectory();
    if (path === null) {
      debugLog("App runScan cancelled (no path)");
      return;
    }
    debugLog(`App runScan initiated path=${path}`);
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
  };

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
          <DiskUsageView />
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
