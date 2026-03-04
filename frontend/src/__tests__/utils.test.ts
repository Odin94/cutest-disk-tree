import { describe, it, expect } from "vitest";
import { humanSize, basename, progressTopLevelFolder } from "../utils";

describe("humanSize", () => {
  it("formats bytes", () => {
    expect(humanSize(0)).toBe("0 B");
    expect(humanSize(512)).toBe("512 B");
    expect(humanSize(1023)).toBe("1023 B");
  });

  it("formats kilobytes", () => {
    expect(humanSize(1024)).toBe("1.0 KB");
    expect(humanSize(1536)).toBe("1.5 KB");
    expect(humanSize(1024 * 1023)).toBe("1023.0 KB");
  });

  it("formats megabytes", () => {
    expect(humanSize(1024 * 1024)).toBe("1.0 MB");
    expect(humanSize(1024 * 1024 * 500)).toBe("500.0 MB");
  });

  it("formats gigabytes", () => {
    expect(humanSize(1024 * 1024 * 1024)).toBe("1.0 GB");
    expect(humanSize(1024 * 1024 * 1024 * 2.5)).toBe("2.5 GB");
  });

  it("formats terabytes", () => {
    expect(humanSize(1024 * 1024 * 1024 * 1024)).toBe("1.0 TB");
    expect(humanSize(1024 * 1024 * 1024 * 1024 * 3.7)).toBe("3.7 TB");
  });
});

describe("basename", () => {
  it("returns last segment for unix paths", () => {
    expect(basename("/usr/local/bin")).toBe("bin");
    expect(basename("/home/user/file.txt")).toBe("file.txt");
  });

  it("returns last segment for windows paths", () => {
    expect(basename("C:\\Users\\test\\file.txt")).toBe("file.txt");
    expect(basename("C:\\Program Files\\app")).toBe("app");
  });

  it("returns the path itself if no separator", () => {
    expect(basename("file.txt")).toBe("file.txt");
    expect(basename("single")).toBe("single");
  });

  it("handles root paths", () => {
    expect(basename("/")).toBe("");
  });
});

describe("progressTopLevelFolder", () => {
  it("returns null when rootPath is null", () => {
    expect(progressTopLevelFolder(null, "/some/path")).toBeNull();
  });

  it("returns null when rootPath is empty", () => {
    expect(progressTopLevelFolder("", "/some/path")).toBeNull();
  });

  it("returns basename of root when currentPath is null", () => {
    expect(progressTopLevelFolder("C:\\data", null)).toBe("data");
  });

  it("returns basename of root when currentPath is empty", () => {
    expect(progressTopLevelFolder("C:\\data", "")).toBe("data");
  });

  it("extracts top-level folder from currentPath under root (windows)", () => {
    expect(
      progressTopLevelFolder("C:\\data", "C:\\data\\subfolder\\deep\\file.txt")
    ).toBe("subfolder");
  });

  it("extracts top-level folder from currentPath under root (unix)", () => {
    expect(
      progressTopLevelFolder("/data", "/data/subfolder/deep/file.txt")
    ).toBe("subfolder");
  });

  it("returns relative itself when currentPath is direct child", () => {
    expect(progressTopLevelFolder("C:\\data", "C:\\data\\myfile.txt")).toBe(
      "myfile.txt"
    );
  });

  it("handles trailing separator on root", () => {
    expect(
      progressTopLevelFolder("C:\\data\\", "C:\\data\\sub\\file.txt")
    ).toBe("sub");
  });

  it("returns relative path when currentPath is not under root", () => {
    expect(
      progressTopLevelFolder("C:\\data", "D:\\other\\deep\\file.txt")
    ).toBe("D:");
  });
});
