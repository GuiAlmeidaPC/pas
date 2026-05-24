import { readFile } from "node:fs/promises";
import { test } from "node:test";
import assert from "node:assert/strict";

test("AI panel does not persist API keys in browser storage", async () => {
  const source = await readFile(new URL("./AIChatPanel.tsx", import.meta.url), "utf8");

  assert.match(source, /invoke\("set_ai_config"/);
  assert.match(source, /pas\.ai_config_public/);
  assert.doesNotMatch(source, /localStorage\.setItem\("pas\.ai_config"/);
  assert.doesNotMatch(source, /fetch\(/);
});

test("dataset viewer ignores stale page responses", async () => {
  const source = await readFile(new URL("./DatasetViewer.tsx", import.meta.url), "utf8");

  assert.match(source, /requestSeqRef/);
  assert.match(source, /requestSeq !== requestSeqRef\.current/);
});
