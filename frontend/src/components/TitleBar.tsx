import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { TabId } from "../views/FileFindingView";

type CategoryId = "disk" | "find";

type TitleBarProps = {
  category: CategoryId;
  activeTab: TabId;
  onNavigate: (category: CategoryId, tab?: TabId) => void;
};

export const TitleBar = ({ category, activeTab, onNavigate }: TitleBarProps) => {
  const [menuOpen, setMenuOpen] = useState(false);
  const [maximized, setMaximized] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const appWindow = getCurrentWindow();

  useEffect(() => {
    appWindow.isMaximized().then(setMaximized).catch(() => {});
    const unlisten = appWindow.onResized(() => {
      appWindow.isMaximized().then(setMaximized).catch(() => {});
    });
    return () => { unlisten.then((fn) => fn()).catch(() => {}); };
  }, []);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const label = category === "disk" ? "Disk Usage" : activeTab === "folders" ? "Largest Folders" : "Find Files";

  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="titlebar-left" ref={menuRef}>
        <button
          type="button"
          className={`titlebar-menu-btn${menuOpen ? " active" : ""}`}
          onClick={() => setMenuOpen((o) => !o)}
          title="View"
        >
          ☰
        </button>
        {menuOpen && (
          <div className="titlebar-menu-dropdown">
            <button
              type="button"
              className={category === "disk" ? "active" : ""}
              onClick={() => { onNavigate("disk"); setMenuOpen(false); }}
            >
              Disk Usage
            </button>
            <hr className="titlebar-menu-separator" />
            <button
              type="button"
              className={category === "find" && activeTab === "find" ? "active" : ""}
              onClick={() => { onNavigate("find", "find"); setMenuOpen(false); }}
            >
              Find Files
            </button>
            <button
              type="button"
              className={category === "find" && activeTab === "folders" ? "active" : ""}
              onClick={() => { onNavigate("find", "folders"); setMenuOpen(false); }}
            >
              Largest Folders
            </button>
          </div>
        )}
        <span className="titlebar-title" data-tauri-drag-region>{label}</span>
      </div>

      <div className="titlebar-controls">
        <button
          type="button"
          className="titlebar-btn titlebar-minimize"
          onClick={() => appWindow.minimize()}
          title="Minimize"
        >
          ─
        </button>
        <button
          type="button"
          className="titlebar-btn titlebar-maximize"
          onClick={() => appWindow.toggleMaximize()}
          title={maximized ? "Restore" : "Maximize"}
        >
          {maximized ? "❐" : "□"}
        </button>
        <button
          type="button"
          className="titlebar-btn titlebar-close"
          onClick={() => appWindow.close()}
          title="Close"
        >
          ✕
        </button>
      </div>
    </div>
  );
};
