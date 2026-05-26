import { describe, it, expect } from "vitest";
import { parseEditBlocks } from "../ai/editProtocol";

describe("parseEditBlocks", () => {
  it("returns empty array when no pas-edit blocks present", () => {
    expect(parseEditBlocks("just text\n```sas\ndata x;\n```\n")).toEqual([]);
  });

  it("parses a single patch block with one hunk", () => {
    const md = [
      '```pas-edit path="programs/foo.sas" mode="patch"',
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
      path: "programs/foo.sas",
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
      '```pas-edit path="a.sas" mode="patch"',
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
      '```pas-edit path="programs/new.sas" mode="create"',
      "data clean; set raw; run;",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toEqual({
      kind: "create",
      path: "programs/new.sas",
      contents: "data clean; set raw; run;",
    });
  });

  it("parses replace blocks", () => {
    const md = [
      '```pas-edit path="big.sas" mode="replace"',
      "line1",
      "line2",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toEqual({
      kind: "replace",
      path: "big.sas",
      contents: "line1\nline2",
    });
  });

  it("returns an error edit when path attribute is missing", () => {
    const md = '```pas-edit mode="create"\nx\n```';
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error", reason: /path/ });
  });

  it("returns an error edit when mode is unknown", () => {
    const md = '```pas-edit path="a.sas" mode="rewrite"\nx\n```';
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error", reason: /mode/ });
  });

  it("returns an error edit for malformed patch markers", () => {
    const md = [
      '```pas-edit path="a.sas" mode="patch"',
      "<<<<<<< SEARCH",
      "no separator or closer here",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error" });
  });

  it("handles CRLF line endings in patch bodies", () => {
    const md = [
      '```pas-edit path="a.sas" mode="patch"',
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
      path: "a.sas",
      hunks: [{ search: "old", replace: "new" }],
    });
  });
});
