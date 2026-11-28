// Generated oracle corpus for the Worker HTML5 entity decoder.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { HTML5_ENTITIES } from "../src/html5-entities.ts";
import { decodeHtmlEntities } from "../src/lib.ts";

interface Corpus {
  oracle: string;
  entity_key_count: number;
  cases: Array<{ input: string; expected: string }>;
}

const corpus = JSON.parse(readFileSync(new URL("./html5-entity-corpus.json", import.meta.url), "utf8")) as Corpus;

test("decodeHtmlEntities matches the generated Python html.unescape corpus", () => {
  assert.equal(corpus.oracle, "Python html.unescape");
  assert.equal(corpus.entity_key_count, HTML5_ENTITIES.size, "all HTML5 table keys are covered");

  for (const { input, expected } of corpus.cases) {
    assert.equal(decodeHtmlEntities(input), expected, `input ${JSON.stringify(input)}`);
  }
});
