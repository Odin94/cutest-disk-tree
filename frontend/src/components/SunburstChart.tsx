import { useMemo, memo } from "react";
import { ResponsiveSunburst } from "@nivo/sunburst";
import type { DiskTreeNode } from "../utils/diskTree";
import { humanSize } from "../utils";
import { debugLog } from "../api";

type NivoNode = {
  id: string;
  value: number;
  children?: NivoNode[];
};

const toNivoNode = (node: DiskTreeNode): NivoNode => {
  const base = { id: node.path, value: node.size };
  const isOther = node.path.endsWith("__other");
  if (!isOther && node.children != null && node.children.length > 0) {
    return { ...base, children: node.children.map(toNivoNode) };
  }
  return base;
};

type SunburstChartProps = {
  tree: DiskTreeNode | null;
  totalSize: number;
  onHover: (path: string | null) => void;
};

const SUNBURST_COLORS = [
  "#22c55e",
  "#eab308",
  "#f97316",
  "#3b82f6",
  "#a855f7",
  "#06b6d4",
  "#84cc16",
  "#ef4444",
  "#64748b",
];

export const SunburstChart = memo(function SunburstChart({
  tree,
  totalSize,
  onHover,
}: SunburstChartProps) {
  const data = useMemo(() => {
    if (tree == null) return null;
    return toNivoNode(tree);
  }, [tree]);

  if (data == null) {
    return null;
  }

  return (
    <div className="sunburst-wrap">
      <ResponsiveSunburst
        data={data}
        margin={{ top: 20, right: 20, bottom: 20, left: 20 }}
        id="id"
        value="value"
        cornerRadius={2}
        borderWidth={1}
        borderColor={{ theme: "background" }}
        colors={SUNBURST_COLORS}
        childColor={{
          from: "color",
          modifiers: [["brighter", 0.2]],
        }}
        enableArcLabels={false}
        arcLabelsSkipAngle={12}
        onMouseEnter={(node) => {
          const t0 = performance.now();
          const path = typeof node.id === "string" ? node.id : null;
          debugLog(`sidebar_prof sunburst onMouseEnter path=${path ?? "null"} t=${t0.toFixed(1)}`);
          onHover(path);
        }}
        onMouseLeave={() => {
          const t0 = performance.now();
          debugLog(`sidebar_prof sunburst onMouseLeave t=${t0.toFixed(1)}`);
          onHover(null);
        }}
        tooltip={({ id, value }) => (
          <div className="sunburst-tooltip">
            <span className="sunburst-tooltip-name">{id}</span>
            <span className="sunburst-tooltip-size">
              {value != null ? humanSize(value) : ""}
            </span>
          </div>
        )}
      />
      <div className="sunburst-center">
        <span className="sunburst-center-value">
          {humanSize(totalSize)}
        </span>
      </div>
    </div>
  );
});
