import { describe, it, expect } from "vitest";
import { buildDiskTree, getDirectChildren } from "../utils/diskTree";
import type { ScanResult } from "../types";

const makeScan = (overrides?: Partial<ScanResult>): ScanResult => ({
  roots: ["C:\\data"],
  files: [
    { path: "C:\\data\\a.txt", size: 100, file_key: { dev: 1, ino: 1 } },
    { path: "C:\\data\\b.txt", size: 200, file_key: { dev: 1, ino: 2 } },
    { path: "C:\\data\\sub\\c.txt", size: 300, file_key: { dev: 1, ino: 3 } },
    { path: "C:\\data\\sub\\deep\\d.txt", size: 400, file_key: { dev: 1, ino: 4 } },
  ],
  folder_sizes: {
    "C:\\data": 1000,
    "C:\\data\\sub": 700,
    "C:\\data\\sub\\deep": 400,
  },
  ...overrides,
});

describe("buildDiskTree", () => {
  it("returns null when root has no folder size entry", () => {
    const scan = makeScan({ folder_sizes: {} });
    expect(buildDiskTree(scan)).toBeNull();
  });

  it("builds a tree from the first root by default", () => {
    const scan = makeScan();
    const tree = buildDiskTree(scan);
    expect(tree).not.toBeNull();
    expect(tree!.path).toBe("C:\\data");
    expect(tree!.size).toBe(1000);
    expect(tree!.name).toBe("data");
  });

  it("uses explicit startPath when provided", () => {
    const scan = makeScan();
    const tree = buildDiskTree(scan, "C:\\data\\sub");
    expect(tree).not.toBeNull();
    expect(tree!.path).toBe("C:\\data\\sub");
    expect(tree!.size).toBe(700);
  });

  it("respects maxDepth option", () => {
    const scan = makeScan();
    const tree = buildDiskTree(scan, undefined, { maxDepth: 0 });
    expect(tree).not.toBeNull();
    expect(tree!.children).toBeUndefined();
  });

  it("includes folder and file children sorted by size", () => {
    const scan = makeScan();
    const tree = buildDiskTree(scan, undefined, { maxDepth: 1 });
    expect(tree).not.toBeNull();
    expect(tree!.children).toBeDefined();
    const names = tree!.children!.map((c) => c.name);
    expect(names).toContain("sub");
    expect(names).toContain("b.txt");
    expect(names).toContain("a.txt");
    expect(tree!.children![0].size).toBeGreaterThanOrEqual(
      tree!.children![tree!.children!.length - 1].size
    );
  });

  it("respects maxChildrenPerNode option", () => {
    const scan = makeScan();
    const tree = buildDiskTree(scan, undefined, {
      maxChildrenPerNode: 1,
      maxDepth: 1,
    });
    expect(tree).not.toBeNull();
    expect(tree!.children).toBeDefined();
    expect(tree!.children!.length).toBe(1);
    expect(tree!.children![0].name).toBe("sub");
  });

  it("recursively expands folder children", () => {
    const scan = makeScan();
    const tree = buildDiskTree(scan, undefined, { maxDepth: 3 });
    expect(tree).not.toBeNull();
    const sub = tree!.children?.find((c) => c.name === "sub");
    expect(sub).toBeDefined();
    const deep = sub!.children?.find((c) => c.name === "deep");
    expect(deep).toBeDefined();
    expect(deep!.size).toBe(400);
  });

  it("handles empty roots array by using empty string", () => {
    const scan = makeScan({ roots: [], folder_sizes: { "": 500 } });
    const tree = buildDiskTree(scan);
    expect(tree).not.toBeNull();
    expect(tree!.size).toBe(500);
  });
});

describe("getDirectChildren", () => {
  it("returns folder and file children sorted by size", () => {
    const scan = makeScan();
    const children = getDirectChildren(scan, "C:\\data");
    expect(children.length).toBeGreaterThanOrEqual(3);
    expect(children[0].size).toBeGreaterThanOrEqual(
      children[children.length - 1].size
    );
    const folderChild = children.find((c) => c.name === "sub");
    expect(folderChild).toBeDefined();
    expect(folderChild!.isFolder).toBe(true);
    const fileChild = children.find((c) => c.name === "a.txt");
    expect(fileChild).toBeDefined();
    expect(fileChild!.isFolder).toBe(false);
  });

  it("limits results to the specified count", () => {
    const scan = makeScan();
    const children = getDirectChildren(scan, "C:\\data", 2);
    expect(children.length).toBe(2);
  });

  it("returns empty array for a path with no children", () => {
    const scan = makeScan();
    const children = getDirectChildren(scan, "C:\\data\\sub\\deep");
    const folders = children.filter((c) => c.isFolder);
    expect(folders.length).toBe(0);
  });

  it("returns children of a subfolder", () => {
    const scan = makeScan();
    const children = getDirectChildren(scan, "C:\\data\\sub");
    const names = children.map((c) => c.name);
    expect(names).toContain("deep");
    expect(names).toContain("c.txt");
    expect(names).not.toContain("a.txt");
  });
});
