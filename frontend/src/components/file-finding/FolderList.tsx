import { motion } from "framer-motion";
import { humanSize } from "../../utils";

type FolderListProps = {
  folders: { path: string; size: number }[];
};

export const FolderList = ({ folders }: FolderListProps) => (
  <motion.div
    initial={{ y: 20, opacity: 0 }}
    animate={{ y: 0, opacity: 1 }}
    transition={{ duration: 0.5, delay: 0.25 }}
    className="glass-strong rounded-2xl overflow-hidden"
  >
    <div className="px-5 py-3 border-b border-border/50 text-sm font-semibold text-muted-foreground">
      Largest folders
    </div>
    <div className="max-h-[60vh] overflow-y-auto">
      {folders.slice(0, 100).map(({ path, size }) => (
        <motion.div
          key={path}
          layout
          initial={{ opacity: 0, x: -6 }}
          animate={{ opacity: 1, x: 0 }}
          className="flex items-center gap-2 px-5 py-3 hover:bg-muted/40 transition-colors duration-200 border-b border-border/30 last:border-b-0"
        >
          <div className="flex-1 min-w-0">
            <span className="text-sm text-foreground truncate block" title={path}>
              {path}
            </span>
          </div>
          <div className="flex-shrink-0 w-20 text-right">
            <span className="text-sm text-muted-foreground font-variant-numeric tabular-nums">
              {humanSize(size)}
            </span>
          </div>
        </motion.div>
      ))}
    </div>
  </motion.div>
);
