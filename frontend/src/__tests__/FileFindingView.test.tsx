import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { getFileName, FileFindingView } from "../views/FileFindingView";
import type { ScanResult, ScanProgress } from "../types";

vi.mock("../api", () => ({
  debugLog: vi.fn(),
  findFiles: vi.fn(() =>
    Promise.resolve({ items: [], nextOffset: null })
  ),
}));

const makeScan = (): ScanResult => ({
  roots: ["C:\\data"],
  files: [],
  files_count: 300,
  folder_sizes: {
    "C:\\data": 300,
  },
});

describe("getFileName", () => {
  it("extracts name from unix path", () => {
    expect(getFileName("/home/user/file.txt")).toBe("file.txt");
  });

  it("extracts name from windows path", () => {
    expect(getFileName("C:\\Users\\test\\file.txt")).toBe("file.txt");
  });

  it("returns input for plain name", () => {
    expect(getFileName("file.txt")).toBe("file.txt");
  });

  it("handles empty string", () => {
    expect(getFileName("")).toBe("");
  });
});

describe("FileFindingView", () => {
  const defaultProps = {
    result: null as ScanResult | null,
    loading: false,
    error: null as string | null,
    progress: null as ScanProgress | null,
    onScan: vi.fn(),
    onCancelScan: vi.fn(),
    activeTab: "find" as const,
    onTabChange: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders empty state when no result", () => {
    render(<FileFindingView {...defaultProps} />);
    expect(
      screen.getByText(/Click.*Scan filesystem.*to start/i)
    ).toBeInTheDocument();
  });

  it("renders scan button", () => {
    render(<FileFindingView {...defaultProps} />);
    const btn = screen.getByRole("button", { name: /scan filesystem/i });
    expect(btn).toBeInTheDocument();
    expect(btn).not.toBeDisabled();
  });

  it("disables scan button when loading", () => {
    render(<FileFindingView {...defaultProps} loading={true} />);
    const btn = screen.getByRole("button", { name: /scanning/i });
    expect(btn).toBeDisabled();
  });

  it("shows error message", () => {
    render(
      <FileFindingView {...defaultProps} error="something went wrong" />
    );
    expect(screen.getByText("something went wrong")).toBeInTheDocument();
  });

  it("shows progress panel when loading with progress", () => {
    const progress: ScanProgress = { files_count: 12345 };
    render(
      <FileFindingView {...defaultProps} loading={true} progress={progress} />
    );
    const matches = screen.getAllByText(/12[.,]345 files scanned/);
    expect(matches.length).toBeGreaterThanOrEqual(1);
  });

  it("shows cancel button when loading and onCancelScan provided", () => {
    render(
      <FileFindingView
        {...defaultProps}
        loading={true}
        progress={{ files_count: 100 }}
      />
    );
    const cancelBtn = screen.getByRole("button", { name: /cancel scan/i });
    expect(cancelBtn).toBeInTheDocument();
  });

  it("calls onScan when scan button is clicked", async () => {
    const user = userEvent.setup();
    render(<FileFindingView {...defaultProps} />);
    const btn = screen.getByRole("button", { name: /scan filesystem/i });
    await user.click(btn);
    expect(defaultProps.onScan).toHaveBeenCalledOnce();
  });

  it("renders summary with scanned roots when result is provided", () => {
    render(<FileFindingView {...defaultProps} result={makeScan()} />);
    expect(screen.getByText("300")).toBeInTheDocument();
    expect(screen.getByText("files indexed")).toBeInTheDocument();
  });

  it("shows find files content when activeTab is find", () => {
    render(<FileFindingView {...defaultProps} result={makeScan()} activeTab="find" />);
    expect(screen.getByPlaceholderText(/search your cozy file collection/i)).toBeInTheDocument();
  });

  it("shows folder list when activeTab is folders", () => {
    render(<FileFindingView {...defaultProps} result={makeScan()} activeTab="folders" />);
    expect(screen.getByText("C:\\data")).toBeInTheDocument();
  });
});
