import { motion } from "framer-motion";
import { FileText, Image, Music, Video, Archive, File, FolderOpen, Terminal } from "lucide-react";

type FileCategory =
  | "all"
  | "audio"
  | "document"
  | "video"
  | "image"
  | "executable"
  | "compressed"
  | "config"
  | "folder"
  | "other";

type FilterBarProps = {
  activeCategory: FileCategory;
  onCategoryChange: (cat: FileCategory) => void;
  disabled?: boolean;
};

const categories: { value: FileCategory; label: string; icon: typeof FileText; color: string }[] = [
  { value: "all", label: "All", icon: FolderOpen, color: "bg-muted" },
  { value: "document", label: "Docs", icon: FileText, color: "bg-sky" },
  { value: "image", label: "Images", icon: Image, color: "bg-peach" },
  { value: "audio", label: "Audio", icon: Music, color: "bg-lavender" },
  { value: "video", label: "Video", icon: Video, color: "bg-rose" },
  { value: "compressed", label: "Archives", icon: Archive, color: "bg-cream" },
  { value: "executable", label: "Exec", icon: Terminal, color: "bg-mint" },
  { value: "config", label: "Config", icon: File, color: "bg-muted" },
  { value: "folder", label: "Folders", icon: FolderOpen, color: "bg-muted" },
  { value: "other", label: "Other", icon: File, color: "bg-muted" },
];

export const FilterBar = ({
  activeCategory,
  onCategoryChange,
  disabled = false,
}: FilterBarProps) => (
  <motion.div
    initial={{ y: 10, opacity: 0 }}
    animate={{ y: 0, opacity: 1 }}
    transition={{ duration: 0.4, delay: 0.15 }}
    className="flex items-center gap-3 flex-wrap"
  >
    <div className="flex items-center gap-1.5 glass rounded-2xl p-1.5">
      {categories.map((cat) => {
        const Icon = cat.icon;
        const isActive = activeCategory === cat.value;
        return (
          <motion.button
            key={cat.label}
            whileHover={{ scale: 1.05 }}
            whileTap={{ scale: 0.95 }}
            onClick={() => onCategoryChange(cat.value)}
            disabled={disabled}
            type="button"
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-sm font-medium transition-all duration-300 disabled:opacity-60 cursor-pointer ${
              isActive ? `${cat.color} text-foreground shadow-cozy` : "text-muted-foreground hover:bg-muted/50 hover:text-foreground"
            }`}
          >
            <Icon className="w-3.5 h-3.5" />
            <span className="hidden sm:inline">{cat.label}</span>
          </motion.button>
        );
      })}
    </div>
  </motion.div>
);
