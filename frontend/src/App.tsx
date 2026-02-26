import { useState } from "react";
import { scanDirectory, pickDirectory } from "./api";
import type { ScanResult } from "./types";
import { humanSize } from "./utils";
import "./App.css";

type TabId = "folders" | "files" | "duplicates";

const App = () => {
  const [result, setResult] = useState<ScanResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("folders");

  const runScan = async () => {
    const path = await pickDirectory();
    if (path === null) return;
    setLoading(true);
    setError(null);
    try {
      const data = await scanDirectory(path);
      setResult(data);
      setActiveTab("folders");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  const uniqueCount =
    result === null
      ? 0
      : new Set(result.files.map((f) => `${f.file_key.dev}:${f.file_key.ino}`)).size;
  const uniqueSize =
    result === null
      ? 0
      : (() => {
          const seen = new Set<string>();
          let sum = 0;
          for (const f of result.files) {
            const k = `${f.file_key.dev}:${f.file_key.ino}`;
            if (seen.has(k)) continue;
            seen.add(k);
            sum += f.size;
          }
          return sum;
        })();

  const folderList =
    result === null
      ? []
      : Object.entries(result.folder_sizes)
          .map(([path, size]) => ({ path, size }))
          .sort((a, b) => b.size - a.size);

  const largestFiles =
    result === null
      ? []
      : [...result.files]
          .sort((a, b) => b.size - a.size)
          .slice(0, 200);

  return (
    <div className="app">
      <header className="header">
        <h1>Cutest Disk Tree</h1>
        <button
          type="button"
          className="primary"
          onClick={runScan}
          disabled={loading}
        >
          {loading ? "Scanning…" : "Choose folder to scan"}
        </button>
      </header>

      {error ? (
        <div className="error">{error}</div>
      ) : null}

      {result !== null && !loading ? (
        <>
          <section className="summary">
            <div className="summary-row">
              <span className="label">Root:</span>
              <span className="path">{result.root}</span>
            </div>
            <div className="summary-row">
              <span className="label">File entries:</span>
              <span>{result.files.length.toLocaleString()}</span>
            </div>
            <div className="summary-row">
              <span className="label">Unique files (hard links deduped):</span>
              <span>
                {uniqueCount.toLocaleString()} ({humanSize(uniqueSize)})
              </span>
            </div>
          </section>

          <div className="tabs">
            {(["folders", "files", "duplicates"] as const).map((tab) => (
              <button
                key={tab}
                type="button"
                className={activeTab === tab ? "tab active" : "tab"}
                onClick={() => setActiveTab(tab)}
              >
                {tab === "folders"
                  ? "Largest folders"
                  : tab === "files"
                    ? "Largest files"
                    : "Duplicates"}
              </button>
            ))}
          </div>

          <div className="panel">
            {activeTab === "folders" ? (
              <ul className="list folder-list">
                {folderList.slice(0, 100).map(({ path, size }) => (
                  <li key={path} className="list-item">
                    <span className="size">{humanSize(size)}</span>
                    <span className="path">{path}</span>
                  </li>
                ))}
              </ul>
            ) : activeTab === "files" ? (
              <ul className="list file-list">
                {largestFiles.map((f) => (
                  <li key={f.path} className="list-item">
                    <span className="size">{humanSize(f.size)}</span>
                    <span className="path">{f.path}</span>
                  </li>
                ))}
              </ul>
            ) : (
              <div className="placeholder">
                <p>Duplicate detection will group files by content hash.</p>
                <p>Hashing is not implemented yet; run from CLI to index, then add hashing in a later step.</p>
              </div>
            )}
          </div>
        </>
      ) : loading ? null : (
        <div className="empty">
          Click &quot;Choose folder to scan&quot; to start.
        </div>
      )}
    </div>
  );
};

export default App;
