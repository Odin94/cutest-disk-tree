import { useState, useRef, useEffect, useLayoutEffect, useCallback, memo, useMemo } from "react";
import { createPortal } from "react-dom";
import { motion, AnimatePresence } from "framer-motion";
import {
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  FileText,
  Image,
  Music,
  Video,
  Code,
  Archive,
  File,
  FolderOpen,
  ExternalLink,
  Copy,
  ClipboardCopy,
  FileIcon,
  Check,
} from "lucide-react";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import type { FileSearchResult } from "../../types";
import { humanSize } from "../../utils";
import { debugLog } from "../../api";

const getFileName = (path: string): string => {
  const segments = path.split(/[/\\]/);
  return segments[segments.length - 1] ?? path;
};

const getExtension = (path: string): string => {
  const name = getFileName(path);
  const ext = name.split(".").pop();
  return ext != null && ext !== name ? ext : "";
};

const getParentPath = (path: string): string => {
  const sep = path.includes("\\") ? "\\" : "/";
  const lastSep = path.lastIndexOf(sep);
  return lastSep <= 0 ? path : path.slice(0, lastSep);
};

const getCategoryFromPath = (path: string): string => {
  const ext = getExtension(path).toLowerCase();
  const docExts = ["pdf", "txt", "md", "doc", "docx", "xls", "xlsx", "csv", "ppt", "pptx"];
  const imgExts = ["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "tiff"];
  const audioExts = ["mp3", "wav", "flac", "m4a", "ogg", "aac", "opus"];
  const videoExts = ["mp4", "mkv", "mov", "avi", "webm", "m4v"];
  const archiveExts = ["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"];
  const codeExts = ["ts", "tsx", "js", "jsx", "py", "rs", "go", "css", "html", "c", "cpp", "h", "java", "kt", "sql", "sh"];
  const configExts = ["cfg", "conf", "ini", "json", "yaml", "yml", "toml", "xml"];
  if (docExts.includes(ext)) return "document";
  if (imgExts.includes(ext)) return "image";
  if (audioExts.includes(ext)) return "audio";
  if (videoExts.includes(ext)) return "video";
  if (archiveExts.includes(ext)) return "archive";
  if (codeExts.includes(ext)) return "code";
  if (configExts.includes(ext)) return "config";
  return "other";
};

type SortField = "name" | "path" | "size" | "type";
type SortDir = "asc" | "desc";

type FileTableProps = {
  files: FileSearchResult[];
  query?: string;
  searchLoading?: boolean;
  hasMore?: boolean;
  loadingMore?: boolean;
  onLoadMore?: () => void;
};

const categoryIcon: Record<string, typeof FileText> = {
  document: FileText,
  image: Image,
  audio: Music,
  video: Video,
  code: Code,
  archive: Archive,
  config: File,
  folder: FolderOpen,
  other: File,
};

const categoryColor: Record<string, string> = {
  document: "bg-sky/50 text-accent-foreground",
  image: "bg-peach/50 text-foreground",
  audio: "bg-lavender/50 text-secondary-foreground",
  video: "bg-rose/50 text-foreground",
  code: "bg-mint/50 text-accent-foreground",
  archive: "bg-cream/50 text-foreground",
  config: "bg-muted text-muted-foreground",
  folder: "bg-muted text-muted-foreground",
  other: "bg-muted text-muted-foreground",
};

type FileRowFile = { path: string; size: number };

const menuItems = [
  {
    group: "open",
    items: [
      {
        label: "Open File",
        icon: ExternalLink,
        action: async (f: FileRowFile) => {
          await openPath(f.path);
          toast.success(`Opening ${getFileName(f.path)}`);
        },
      },
      {
        label: "Open Path",
        icon: FolderOpen,
        action: async (f: FileRowFile) => {
          await revealItemInDir(f.path);
          toast.success(`Opening ${getParentPath(f.path)}`);
        },
      },
    ],
  },
  {
    group: "copy",
    items: [
      {
        label: "Copy File Name",
        icon: FileIcon,
        action: (f: FileRowFile) => {
          navigator.clipboard.writeText(getFileName(f.path));
          toast.success("File name copied!");
        },
      },
      {
        label: "Copy File",
        icon: Copy,
        action: (f: FileRowFile) => {
          navigator.clipboard.writeText(f.path);
          toast.success("File copied!");
        },
      },
      {
        label: "Copy Path",
        icon: ClipboardCopy,
        action: (f: FileRowFile) => {
          navigator.clipboard.writeText(f.path);
          toast.success("Path copied!");
        },
      },
    ],
  },
];

const HighlightedText = ({
  text,
  query,
  className,
}: {
  text: string;
  query?: string;
  className?: string;
}) => {
  if (!query?.trim()) {
    return <span className={className}>{text}</span>;
  }
  try {
    const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const regex = new RegExp(`(${escaped})`, "gi");
    const parts = text.split(regex);
    if (parts.length === 1) {
      return <span className={className}>{text}</span>;
    }
    return (
      <span className={className}>
        {parts.map((part, i) =>
          regex.test(part) ? (
            <mark key={i} className="bg-primary/25 text-foreground rounded-sm px-0.5">
              {part}
            </mark>
          ) : (
            <span key={i}>{part}</span>
          )
        )}
      </span>
    );
  } catch {
    return <span className={className}>{text}</span>;
  }
};

type FileRowProps = {
  file: FileSearchResult;
  index: number;
  query?: string;
  useSimpleAnimation?: boolean;
};

const FileRow = memo(({ file, index, query, useSimpleAnimation = false }: FileRowProps) => {
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(null);
  const [selectedLabel, setSelectedLabel] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const name = getFileName(file.path);
  const ext = getExtension(file.path);
  const category = file.kind === "folder" ? "folder" : getCategoryFromPath(file.path);
  const Icon = categoryIcon[category] ?? File;
  const colorCls = categoryColor[category] ?? categoryColor.other;

  const handleItemClick = useCallback(
    (label: string, action: (f: FileRowFile) => void | Promise<void>) => {
      setSelectedLabel(label);
      Promise.resolve(action(file))
        .then(() => {
          setTimeout(() => {
            setMenuOpen(false);
            setSelectedLabel(null);
          }, 450);
        })
        .catch(() => {
          setMenuOpen(false);
          setSelectedLabel(null);
        });
    },
    [file]
  );

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [menuOpen]);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    setMenuPos({ x: e.clientX, y: e.clientY });
    setMenuOpen((prev) => !prev);
  };

  const handleDoubleClick = async () => {
    await revealItemInDir(file.path);
    toast.success(`Opening ${getParentPath(file.path)}`);
  };

  const rowClassName =
    "relative flex items-center gap-2 px-5 py-3 hover:bg-muted/40 transition-colors duration-200 border-b border-border/30 last:border-b-0 group cursor-default select-text";

  return useSimpleAnimation ? (
    <div onContextMenu={handleContextMenu} onDoubleClick={handleDoubleClick} className={rowClassName}>
      <div className="flex-[3] min-w-0 flex items-center gap-3">
        <div
          className={`w-8 h-8 rounded-lg flex items-center justify-center ${colorCls} transition-transform group-hover:scale-110`}
        >
          <Icon className="w-4 h-4" />
        </div>
        <HighlightedText text={name} query={query} className="font-medium text-sm truncate text-foreground" />
      </div>
      <div className="flex-[4] min-w-0 hidden md:flex">
        <HighlightedText text={file.path} query={query} className="text-sm text-muted-foreground truncate" />
      </div>
      <div className="flex-shrink-0 w-20">
        <span className="text-sm text-muted-foreground">{humanSize(file.size)}</span>
      </div>
      <div className="flex-shrink-0 w-24">
        <span className={`text-xs px-2 py-0.5 rounded-full font-medium ${colorCls}`}>
          {file.kind === "folder" ? "folder" : ext ? `.${ext}` : "—"}
        </span>
      </div>

      {menuOpen && menuPos && typeof document !== "undefined"
        ? createPortal(
            <AnimatePresence>
              <motion.div
                ref={menuRef}
                initial={{ opacity: 0, scale: 0.9, y: -5 }}
                animate={{ opacity: 1, scale: 1, y: 0 }}
                exit={{ opacity: 0, scale: 0.9, y: -5 }}
                transition={{ duration: 0.15 }}
                className="fixed z-[9999] glass-strong rounded-xl py-1.5 min-w-[180px] overflow-hidden"
                style={{ left: menuPos.x, top: menuPos.y + 10 }}
                onClick={(e) => e.stopPropagation()}
              >
                {menuItems.map((group, gi) => (
                  <div key={group.group}>
                    {gi > 0 ? <div className="h-px bg-border/50 my-1.5 mx-3" /> : null}
                    {group.items.map((item) => {
                      const ItemIcon = item.icon;
                      const isSelected = selectedLabel === item.label;
                      return (
                        <button
                          key={item.label}
                          type="button"
                          onClick={() => handleItemClick(item.label, item.action)}
                          disabled={selectedLabel !== null}
                          className={`w-full flex items-center gap-2.5 px-4 py-2 text-sm transition-all duration-200 ${
                            isSelected
                              ? "bg-primary/15 text-primary"
                              : "text-foreground hover:bg-primary/10 hover:text-primary"
                          }`}
                        >
                          <AnimatePresence mode="wait">
                            {isSelected ? (
                              <motion.div
                                key="check"
                                initial={{ scale: 0, rotate: -90 }}
                                animate={{ scale: 1, rotate: 0 }}
                                transition={{ type: "spring", stiffness: 400, damping: 15 }}
                              >
                                <Check className="w-3.5 h-3.5 text-primary" />
                              </motion.div>
                            ) : (
                              <motion.div key="icon" exit={{ scale: 0, rotate: 90 }} transition={{ duration: 0.1 }}>
                                <ItemIcon className="w-3.5 h-3.5" />
                              </motion.div>
                            )}
                          </AnimatePresence>
                          {item.label}
                        </button>
                      );
                    })}
                  </div>
                ))}
              </motion.div>
            </AnimatePresence>,
            document.body
          )
        : null}
    </div>
  ) : (
    <motion.div
      layout
      initial={{ opacity: 0, x: -6 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: 6 }}
      transition={{ duration: 0.18, delay: index * 0.015 }}
      onContextMenu={handleContextMenu}
      onDoubleClick={handleDoubleClick}
      className={rowClassName}
    >
      <div className="flex-[3] min-w-0 flex items-center gap-3">
        <div
          className={`w-8 h-8 rounded-lg flex items-center justify-center ${colorCls} transition-transform group-hover:scale-110`}
        >
          <Icon className="w-4 h-4" />
        </div>
        <HighlightedText text={name} query={query} className="font-medium text-sm truncate text-foreground" />
      </div>
      <div className="flex-[4] min-w-0 hidden md:flex">
        <HighlightedText text={file.path} query={query} className="text-sm text-muted-foreground truncate" />
      </div>
      <div className="flex-shrink-0 w-20">
        <span className="text-sm text-muted-foreground">{humanSize(file.size)}</span>
      </div>
      <div className="flex-shrink-0 w-24">
        <span className={`text-xs px-2 py-0.5 rounded-full font-medium ${colorCls}`}>
          {file.kind === "folder" ? "folder" : ext ? `.${ext}` : "—"}
        </span>
      </div>

      {menuOpen && menuPos && typeof document !== "undefined"
        ? createPortal(
            <AnimatePresence>
              <motion.div
                ref={menuRef}
                initial={{ opacity: 0, scale: 0.9, y: -5 }}
                animate={{ opacity: 1, scale: 1, y: 0 }}
                exit={{ opacity: 0, scale: 0.9, y: -5 }}
                transition={{ duration: 0.15 }}
                className="fixed z-[9999] glass-strong rounded-xl py-1.5 min-w-[180px] overflow-hidden"
                style={{ left: menuPos.x, top: menuPos.y + 10 }}
                onClick={(e) => e.stopPropagation()}
              >
                {menuItems.map((group, gi) => (
                  <div key={group.group}>
                    {gi > 0 ? <div className="h-px bg-border/50 my-1.5 mx-3" /> : null}
                    {group.items.map((item) => {
                      const ItemIcon = item.icon;
                      const isSelected = selectedLabel === item.label;
                      return (
                        <button
                          key={item.label}
                          type="button"
                          onClick={() => handleItemClick(item.label, item.action)}
                          disabled={selectedLabel !== null}
                          className={`w-full flex items-center gap-2.5 px-4 py-2 text-sm transition-all duration-200 ${
                            isSelected
                              ? "bg-primary/15 text-primary"
                              : "text-foreground hover:bg-primary/10 hover:text-primary"
                          }`}
                        >
                          <AnimatePresence mode="wait">
                            {isSelected ? (
                              <motion.div
                                key="check"
                                initial={{ scale: 0, rotate: -90 }}
                                animate={{ scale: 1, rotate: 0 }}
                                transition={{ type: "spring", stiffness: 400, damping: 15 }}
                              >
                                <Check className="w-3.5 h-3.5 text-primary" />
                              </motion.div>
                            ) : (
                              <motion.div key="icon" exit={{ scale: 0, rotate: 90 }} transition={{ duration: 0.1 }}>
                                <ItemIcon className="w-3.5 h-3.5" />
                              </motion.div>
                            )}
                          </AnimatePresence>
                          {item.label}
                        </button>
                      );
                    })}
                  </div>
                ))}
              </motion.div>
            </AnimatePresence>,
            document.body
          )
        : null}
    </motion.div>
  );
}, (prev, next) =>
  prev.file.path === next.file.path &&
  prev.file.size === next.file.size &&
  prev.file.kind === next.file.kind &&
  prev.file.file_key?.dev === next.file.file_key?.dev &&
  prev.file.file_key?.ino === next.file.file_key?.ino &&
  prev.index === next.index &&
  prev.query === next.query &&
  prev.useSimpleAnimation === next.useSimpleAnimation
);

const LoadingSpinner = () => (
  <div className="flex items-center justify-center py-6 gap-3">
    <motion.div className="flex gap-1.5" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
      {[0, 1, 2].map((i) => (
        <motion.div
          key={i}
          className="w-2.5 h-2.5 rounded-full bg-primary/60"
          animate={{ y: [0, -8, 0], scale: [1, 1.2, 1] }}
          transition={{ duration: 0.6, repeat: Infinity, delay: i * 0.15, ease: "easeInOut" }}
        />
      ))}
    </motion.div>
    <span className="text-sm text-muted-foreground font-medium">Loading more files…</span>
  </div>
);

export const FileTable = ({
  files,
  query,
  searchLoading = false,
  hasMore = false,
  loadingMore = false,
  onLoadMore,
}: FileTableProps) => {
  const [sortField, setSortField] = useState<SortField>("name");
  const [sortDir, setSortDir] = useState<SortDir>("asc");
  const scrollRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);

  // Render-commit timing: fires synchronously after DOM mutation (before paint).
  // Combined with the ipc_done log in FileFindingView, this shows how long React
  // took to sort + render the new file list.
  const renderStartRef = useRef<number>(0);
  useLayoutEffect(() => {
    renderStartRef.current = performance.now();
  }, [files]);
  // useEffect fires after the browser has painted — gives total "data → visible" latency.
  useEffect(() => {
    if (files.length === 0 && !searchLoading) return; // skip empty/loading state
    const paintMs = Math.round(performance.now() - renderStartRef.current);
    debugLog(`FileTable render_paint_ms=${paintMs} files=${files.length}`);
  }, [files, searchLoading]);

  const toggleSort = (field: SortField) => {
    if (sortField === field) {
      setSortDir(sortDir === "asc" ? "desc" : "asc");
    } else {
      setSortField(field);
      setSortDir("asc");
    }
  };

  const sorted = useMemo(() => [...files].sort((a, b) => {
    const mul = sortDir === "asc" ? 1 : -1;
    if (sortField === "size") return (a.size - b.size) * mul;
    if (sortField === "path") return a.path.localeCompare(b.path) * mul;
    if (sortField === "type") {
      const typeA = a.kind === "folder" ? "folder" : getExtension(a.path).toLowerCase();
      const typeB = b.kind === "folder" ? "folder" : getExtension(b.path).toLowerCase();
      return typeA.localeCompare(typeB) * mul;
    }
    return getFileName(a.path).localeCompare(getFileName(b.path)) * mul;
  }), [files, sortField, sortDir]);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [files, sortField, sortDir]);

  useEffect(() => {
    const sentinel = sentinelRef.current;
    const container = scrollRef.current;
    if (!sentinel || !container || !hasMore || !onLoadMore) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && hasMore && !loadingMore) {
          onLoadMore();
        }
      },
      { root: container, threshold: 0.1 }
    );

    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMore, loadingMore, onLoadMore]);

  const visibleFiles = sorted;

  const SortIcon = ({ field }: { field: SortField }) => {
    if (sortField !== field) return <ArrowUpDown className="w-3.5 h-3.5 opacity-40" />;
    return sortDir === "asc" ? (
      <ArrowUp className="w-3.5 h-3.5 text-primary" />
    ) : (
      <ArrowDown className="w-3.5 h-3.5 text-primary" />
    );
  };

  const columns: { field: SortField; label: string; className: string }[] = [
    { field: "name", label: "Name", className: "flex-[3] min-w-0" },
    { field: "path", label: "Path", className: "flex-[4] min-w-0 hidden md:flex" },
    { field: "size", label: "Size", className: "flex-shrink-0 w-20" },
    { field: "type", label: "Type", className: "flex-shrink-0 w-24" },
  ];

  return (
    <motion.div
      initial={{ y: 20, opacity: 0 }}
      animate={{ y: 0, opacity: 1 }}
      transition={{ duration: 0.5, delay: 0.25 }}
      className="glass-strong rounded-2xl overflow-hidden min-w-0"
    >
      <div className="flex items-center gap-2 px-5 py-3 border-b border-border/50 min-w-0">
        {columns.map((col) => (
          <button
            key={col.field}
            onClick={() => toggleSort(col.field)}
            type="button"
            className={`${col.className} flex items-center gap-1.5 text-sm font-semibold text-muted-foreground hover:text-foreground transition-colors cursor-pointer`}
          >
            {col.label}
            <SortIcon field={col.field} />
          </button>
        ))}
      </div>

      {searchLoading && (
        <div className="h-0.5 overflow-hidden">
          <div className="h-full bg-primary/50 animate-pulse w-full" />
        </div>
      )}
      <div ref={scrollRef} className="h-[60vh] overflow-y-auto overflow-x-hidden min-w-0">
        {!searchLoading && sorted.length === 0 ? (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            className="flex flex-col items-center justify-center h-full text-muted-foreground"
          >
            <span className="text-4xl mb-3">🍂</span>
            <p className="font-display text-lg">No files found</p>
            <p className="text-sm">Try adjusting your search or filters</p>
          </motion.div>
        ) : (
          visibleFiles.map((file, i) => (
            <FileRow
              key={`${file.path}:${file.file_key?.dev ?? "d"}:${file.file_key?.ino ?? "i"}`}
              file={file}
              index={i}
              query={query}
              useSimpleAnimation
            />
          ))
        )}

        {loadingMore && !searchLoading ? <LoadingSpinner /> : null}
        <div ref={sentinelRef} className="h-1" />
      </div>

      <div className="px-5 py-2.5 border-t border-border/50 text-sm text-muted-foreground flex items-center justify-between">
        <span>
          {visibleFiles.length} file{visibleFiles.length !== 1 ? "s" : ""}
          {hasMore ? " (more available…)" : ""}
        </span>
        <a
          href="https://ko-fi.com/odin_dev"
          target="_blank"
          rel="noopener noreferrer"
          className="text-xs hover:text-primary transition-colors duration-200 cursor-pointer"
        >
          ✿ support on Ko-fi
        </a>
      </div>
    </motion.div>
  );
};
