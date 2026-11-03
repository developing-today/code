// Paired-fixture parity tests for extractHtml (src/lib.ts) — #569.
// The SAME fixture file drives the container Extractor's test
// (test/test_extractor_parity.py), so the Worker fallback and the
// container door are pinned to identical {title, links, text} results.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { extractHtml } from "../src/lib.ts";

interface Fixture {
  name: string;
  html: string;
  title: string;
  links: number;
  text: string;
}

const fixtures = JSON.parse(
  readFileSync(new URL("./extractor-fixtures.json", import.meta.url), "utf8"),
) as Fixture[];

for (const f of fixtures) {
  test(`extractHtml parity: ${f.name}`, () => {
    const got = extractHtml(f.html);
    assert.equal(got.title, f.title, "title");
    assert.equal(got.links, f.links, "links");
    assert.equal(got.text, f.text, "text");
  });
}
