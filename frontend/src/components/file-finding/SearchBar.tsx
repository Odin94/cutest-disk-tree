import { Search, Sparkles } from "lucide-react";
import { motion } from "framer-motion";

type SearchBarProps = {
  query: string;
  onQueryChange: (q: string) => void;
  useFuzzySearch: boolean;
  onFuzzySearchChange: (v: boolean) => void;
  disabled?: boolean;
};

const SearchBar = ({
  query,
  onQueryChange,
  useFuzzySearch,
  onFuzzySearchChange,
  disabled = false,
}: SearchBarProps) => (
  <motion.div
    initial={{ y: -20, opacity: 0 }}
    animate={{ y: 0, opacity: 1 }}
    transition={{ duration: 0.5, ease: "easeOut" }}
    className="space-y-2"
  >
    <div className="glass-strong rounded-2xl p-1.5 flex items-center gap-2">
      <div className="flex items-center gap-2 flex-1 px-4">
        <Search className="w-5 h-5 text-primary shrink-0" />
        <input
          type="text"
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          placeholder="Search your cozy file collection..."
          disabled={disabled}
          className="flex-1 bg-transparent border-none outline-none text-foreground placeholder:text-muted-foreground py-3 font-body text-base disabled:opacity-60"
        />
        {query ? (
          <motion.div initial={{ scale: 0 }} animate={{ scale: 1 }} className="text-primary">
            <Sparkles className="w-4 h-4 animate-gentle-bounce" />
          </motion.div>
        ) : null}
      </div>

      <div className="flex items-center gap-1 pr-1">
        <button
          type="button"
          onClick={() => onFuzzySearchChange(false)}
          disabled={disabled}
          className={`px-3 py-2 rounded-xl text-sm font-medium transition-all duration-300 disabled:opacity-60 cursor-pointer ${
            !useFuzzySearch
              ? "bg-primary text-primary-foreground shadow-cozy"
              : "text-muted-foreground hover:bg-muted hover:text-foreground"
          }`}
        >
          <span className="mr-1">🔍</span>
          Exact
        </button>
        <button
          type="button"
          onClick={() => onFuzzySearchChange(true)}
          disabled={disabled}
          className={`px-3 py-2 rounded-xl text-sm font-medium transition-all duration-300 disabled:opacity-60 cursor-pointer ${
            useFuzzySearch
              ? "bg-primary text-primary-foreground shadow-cozy"
              : "text-muted-foreground hover:bg-muted hover:text-foreground"
          }`}
        >
          <span className="mr-1">✨</span>
          Fuzzy
        </button>
      </div>
    </div>
  </motion.div>
);

export default SearchBar;
