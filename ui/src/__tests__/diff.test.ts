import { describe, it, expect } from "vitest";
import { computeHunks } from "../ai/diff";

describe("computeHunks", () => {
  it("returns a single all-add hunk when original is empty", () => {
    const hunks = computeHunks("", "a\nb\n");
    expect(hunks).toHaveLength(1);
    expect(hunks[0].lines.every(l => l.kind === "add")).toBe(true);
    expect(hunks[0].lines.map(l => l.text)).toEqual(["a", "b"]);
  });

  it("returns a single all-del hunk when proposed is empty", () => {
    const hunks = computeHunks("a\nb\n", "");
    expect(hunks[0].lines.every(l => l.kind === "del")).toBe(true);
  });

  it("returns empty hunks when identical", () => {
    expect(computeHunks("a\nb\n", "a\nb\n")).toEqual([]);
  });

  it("collapses unchanged regions and keeps context around changes", () => {
    const before = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
    const after  = "1\n2\n3\n4\nFIVE\n6\n7\n8\n9\n10\n";
    const hunks = computeHunks(before, after);
    expect(hunks).toHaveLength(1);
    const kinds = hunks[0].lines.map(l => l.kind);
    expect(kinds).toContain("del");
    expect(kinds).toContain("add");
    // Should not include far-away unchanged lines 1 and 10
    expect(hunks[0].lines.map(l => l.text)).not.toContain("1");
    expect(hunks[0].lines.map(l => l.text)).not.toContain("10");
  });

  it("numbers context lines with both old and new line numbers", () => {
    const hunks = computeHunks("a\nb\nc\n", "a\nB\nc\n");
    const ctx = hunks[0].lines.filter(l => l.kind === "ctx");
    for (const l of ctx) {
      expect(typeof l.oldLine).toBe("number");
      expect(typeof l.newLine).toBe("number");
    }
  });
});
