import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { DiskUsageView } from "../views/DiskUsageView";
import type { ScanResult } from "../types";

vi.mock("../api", () => ({
  debugLog: vi.fn(),
  buildDiskTreeCached: vi.fn(() => Promise.resolve(null)),
  listCachedTreeDepths: vi.fn(() => Promise.resolve([])),
}));

vi.mock("../components/SunburstChart", () => ({
  SunburstChart: () => <div data-testid="sunburst-chart" />,
}));

const makeScan = (): ScanResult => ({
  roots: ["C:\\data"],
  files: [
    { path: "C:\\data\\a.txt", size: 100, file_key: { dev: 1, ino: 1 } },
  ],
  folder_sizes: { "C:\\data": 100 },
});

describe("DiskUsageView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders empty state when no result", () => {
    render(<DiskUsageView />);
    expect(
      screen.getByText(/no scan data yet/i)
    ).toBeInTheDocument();
  });

  it("renders scan button in empty state", () => {
    render(<DiskUsageView onScan={() => {}} />);
    const btn = screen.getByRole("button", { name: /scan filesystem/i });
    expect(btn).toBeInTheDocument();
  });

  it("renders chart controls when result is provided", () => {
    render(<DiskUsageView result={makeScan()} onScan={() => {}} />);
    const btn = screen.getByRole("button", { name: /rescan filesystem/i });
    expect(btn).toBeInTheDocument();
  });

  it("shows loading state while tree is building", () => {
    render(<DiskUsageView result={makeScan()} />);
    expect(screen.getByText(/building tree/i)).toBeInTheDocument();
  });
});
