import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { DiskUsageSidebar } from "../components/DiskUsageSidebar";
import type { ScanResult } from "../types";
import type { DiskTreeNode } from "../utils/diskTree";

vi.mock("../api", () => ({
  debugLog: vi.fn(),
}));

const makeScan = (): ScanResult => ({
  roots: ["C:\\data"],
  files: [
    { path: "C:\\data\\a.txt", size: 100, file_key: { dev: 1, ino: 1 } },
    { path: "C:\\data\\sub\\b.txt", size: 200, file_key: { dev: 1, ino: 2 } },
  ],
  folder_sizes: {
    "C:\\data": 500,
    "C:\\data\\sub": 300,
  },
});

const makeTree = (): DiskTreeNode => ({
  path: "C:\\data",
  name: "data",
  size: 500,
  children: [
    {
      path: "C:\\data\\sub",
      name: "sub",
      size: 300,
      children: [
        { path: "C:\\data\\sub\\b.txt", name: "b.txt", size: 200 },
      ],
    },
    { path: "C:\\data\\a.txt", name: "a.txt", size: 100 },
  ],
});

describe("DiskUsageSidebar", () => {
  it("renders hint when scan is null", () => {
    render(
      <DiskUsageSidebar scan={null} tree={null} hoveredPath={null} />
    );
    expect(screen.getByText(/look up sidebar/i)).toBeInTheDocument();
  });

  it("renders root name and size when scan is provided", () => {
    render(
      <DiskUsageSidebar
        scan={makeScan()}
        tree={makeTree()}
        hoveredPath={null}
      />
    );
    expect(screen.getByText("data")).toBeInTheDocument();
    expect(screen.getByText("500 B")).toBeInTheDocument();
  });

  it("shows children of the root", () => {
    render(
      <DiskUsageSidebar
        scan={makeScan()}
        tree={makeTree()}
        hoveredPath={null}
      />
    );
    expect(screen.getByText("sub")).toBeInTheDocument();
    expect(screen.getByText("a.txt")).toBeInTheDocument();
  });

  it("displays hovered folder name when hoveredPath is set", () => {
    const { container } = render(
      <DiskUsageSidebar
        scan={makeScan()}
        tree={makeTree()}
        hoveredPath="C:\\data\\sub"
      />
    );
    const title = container.querySelector(".disk-usage-sidebar-title");
    expect(title).not.toBeNull();
    expect(title!.textContent).toBe("sub");
  });

  it("shows path element when hoveredPath differs from root", () => {
    const { container } = render(
      <DiskUsageSidebar
        scan={makeScan()}
        tree={makeTree()}
        hoveredPath="C:\\data\\sub"
      />
    );
    const pathEl = container.querySelector(".disk-usage-sidebar-path");
    expect(pathEl).not.toBeNull();
    expect(pathEl!.textContent).toContain("sub");
  });

  it("does not show path below title when hoveredPath is the root", () => {
    const { container } = render(
      <DiskUsageSidebar
        scan={makeScan()}
        tree={makeTree()}
        hoveredPath={null}
      />
    );
    const pathElement = container.querySelector(
      ".disk-usage-sidebar-path"
    );
    expect(pathElement).toBeNull();
  });
});
