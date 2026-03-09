import { motion } from "framer-motion";
import { FolderPlus, RefreshCw, HardDrive } from "lucide-react";

type IndexingControlsProps = {
  indexedCount: number;
  isIndexing: boolean;
  onIndex: () => void;
};

export const IndexingControls = ({ indexedCount, isIndexing, onIndex }: IndexingControlsProps) => (
  <motion.div
    initial={{ y: 10, opacity: 0 }}
    animate={{ y: 0, opacity: 1 }}
    transition={{ duration: 0.4, delay: 0.1 }}
    className="flex items-center gap-3"
  >
    <motion.button
      whileHover={{ scale: 1.03 }}
      whileTap={{ scale: 0.97 }}
      onClick={onIndex}
      disabled={isIndexing}
      type="button"
      className="glass rounded-2xl px-5 py-2.5 flex items-center gap-2 text-sm font-semibold text-foreground hover:shadow-cozy-lg transition-all duration-300 disabled:opacity-60 cursor-pointer"
    >
      {isIndexing ? (
        <RefreshCw className="w-4 h-4 animate-spin text-primary" />
      ) : (
        <FolderPlus className="w-4 h-4 text-primary" />
      )}
      {isIndexing ? "Scanning…" : "Scan filesystem"}
    </motion.button>

    <div className="glass rounded-2xl px-4 py-2.5 flex items-center gap-2 text-sm text-muted-foreground">
      <HardDrive className="w-4 h-4" />
      <span className="font-medium">{indexedCount.toLocaleString()}</span>
      <span>files indexed</span>
    </div>
  </motion.div>
);
