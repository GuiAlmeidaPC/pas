import { describe, it, expect } from "vitest";
import { parseEditBlocks, applyPatch } from "../ai/editProtocol";

describe("parseEditBlocks", () => {
  it("returns empty array when no pas-edit blocks present", () => {
    expect(parseEditBlocks("just text\n```pas\ndata x;\n```\n")).toEqual([]);
  });

  it("parses a single patch block with one hunk", () => {
    const md = [
      '```pas-edit path="programs/foo.pas" mode="patch"',
      "<<<<<<< SEARCH",
      "data want; set have; run;",
      "=======",
      "data want; set have; where x>0; run;",
      ">>>>>>> REPLACE",
      "```",
    ].join("\n");
    const edits = parseEditBlocks(md);
    expect(edits).toHaveLength(1);
    expect(edits[0]).toMatchObject({
      kind: "patch",
      path: "programs/foo.pas",
      hunks: [
        {
          search: "data want; set have; run;",
          replace: "data want; set have; where x>0; run;",
        },
      ],
    });
  });

  it("parses multiple hunks in one patch block", () => {
    const md = [
      '```pas-edit path="a.pas" mode="patch"',
      "<<<<<<< SEARCH",
      "old1",
      "=======",
      "new1",
      ">>>>>>> REPLACE",
      "<<<<<<< SEARCH",
      "old2",
      "=======",
      "new2",
      ">>>>>>> REPLACE",
      "```",
    ].join("\n");
    const edits = parseEditBlocks(md);
    expect(edits[0]).toMatchObject({
      kind: "patch",
      hunks: [
        { search: "old1", replace: "new1" },
        { search: "old2", replace: "new2" },
      ],
    });
  });

  it("parses create blocks", () => {
    const md = [
      '```pas-edit path="programs/new.pas" mode="create"',
      "data clean; set raw; run;",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toEqual({
      kind: "create",
      path: "programs/new.pas",
      contents: "data clean; set raw; run;",
    });
  });

  it("parses replace blocks", () => {
    const md = [
      '```pas-edit path="big.pas" mode="replace"',
      "line1",
      "line2",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toEqual({
      kind: "replace",
      path: "big.pas",
      contents: "line1\nline2",
    });
  });

  it("returns an error edit when path attribute is missing", () => {
    const md = '```pas-edit mode="create"\nx\n```';
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error", reason: /path/ });
  });

  it("returns an error edit when mode is unknown", () => {
    const md = '```pas-edit path="a.pas" mode="rewrite"\nx\n```';
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error", reason: /mode/ });
  });

  it("returns an error edit for malformed patch markers", () => {
    const md = [
      '```pas-edit path="a.pas" mode="patch"',
      "<<<<<<< SEARCH",
      "no separator or closer here",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error" });
  });

  it("handles CRLF line endings in patch bodies", () => {
    const md = [
      '```pas-edit path="a.pas" mode="patch"',
      "<<<<<<< SEARCH",
      "old",
      "=======",
      "new",
      ">>>>>>> REPLACE",
      "```",
    ].join("\r\n");
    const edits = parseEditBlocks(md);
    expect(edits[0]).toMatchObject({
      kind: "patch",
      path: "a.pas",
      hunks: [{ search: "old", replace: "new" }],
    });
  });
});

describe("applyPatch", () => {
  it("applies a single hunk", () => {
    const r = applyPatch("a\nb\nc\n", [{ search: "b", replace: "BB" }]);
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value).toBe("a\nBB\nc\n");
  });

  it("applies multiple hunks in order", () => {
    const r = applyPatch("a\nb\nc\n", [
      { search: "a", replace: "A" },
      { search: "c", replace: "C" },
    ]);
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value).toBe("A\nb\nC\n");
  });

  it("fails when SEARCH text is not found", () => {
    const r = applyPatch("a\nb\nc\n", [{ search: "missing", replace: "x" }]);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toMatch(/not found/i);
  });

  it("fails when SEARCH text matches more than once (ambiguous)", () => {
    const r = applyPatch("a\na\n", [{ search: "a", replace: "B" }]);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toMatch(/ambiguous|multiple/i);
  });

  it("best-effort variant skips failing hunks but applies the rest", async () => {
    const { applyPatchBestEffort } = await import("../ai/editProtocol");
    const out = applyPatchBestEffort("a\nb\nc\n", [
      { search: "a", replace: "A" },
      { search: "missing", replace: "X" },
      { search: "c", replace: "C" },
    ]);
    expect(out).toBe("A\nb\nC\n");
  });
});
