// Regression tests for #16078: triple backticks inside JSON string
// values must not terminate the fenced Metadata block early.
//
// Oracle: the issue's acceptance — a render/parse round-trip preserves
// JSON strings containing ``` (the parser recognizes the closing
// delimiter line, not backticks inside string contents), while missing
// markers, malformed JSON and invalid fences still fail closed with
// invalid_metadata_block.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  METADATA_END,
  METADATA_START,
  parseMetadataBlock,
  renderMetadataBlock,
} from "../src/metadata.ts";

const BACKTICK_CASES: Array<[string, Record<string, unknown>]> = [
  ["mid-string", { pm_meta: 1, title: "release ``` notes" }],
  ["value is exactly backticks", { pm_meta: 1, title: "```" }],
  ["backticks at value start", { pm_meta: 1, title: "```abc" }],
  ["backticks at value end", { pm_meta: 1, title: "abc```" }],
  ["multiple runs", { pm_meta: 1, title: "a```b```c" }],
  ["backticks around an escaped newline", { pm_meta: 1, title: "a```\n```b" }],
  ["backticks in a link", { pm_meta: 1, url: "https://example.test/```/details" }],
];

for (const [name, block] of BACKTICK_CASES) {
  test(`round-trip preserves triple backticks (${name})`, () => {
    const rendered = renderMetadataBlock(block);
    const parsed = parseMetadataBlock(`caller text\n\n${rendered}\n\ntrailer`);
    assert.deepEqual(parsed.block, block);
    assert.equal(parsed.before, "caller text\n\n");
    assert.equal(parsed.after, "\n\ntrailer");
  });
}

test("missing markers parse to block null", () => {
  assert.deepEqual(parseMetadataBlock("just caller text").block, null);
});

for (const [name, body] of [
  ["end without start", `x ${METADATA_END} y`],
  ["start without end", `${METADATA_START}\n\`\`\`json\n{}\n\`\`\``],
  ["malformed JSON", `${METADATA_START}\n\`\`\`json\n{nope}\n\`\`\`\n${METADATA_END}`],
  ["missing pm_meta", `${METADATA_START}\n\`\`\`json\n{}\n\`\`\`\n${METADATA_END}`],
  ["no fence", `${METADATA_START}\n{}\n${METADATA_END}`],
  ["unclosed fence", `${METADATA_START}\n\`\`\`json\n{"pm_meta": 1}\n${METADATA_END}`],
] as Array<[string, string]>) {
  test(`still fails closed (${name})`, () => {
    assert.throws(() => parseMetadataBlock(body), (error: unknown) => {
      assert.equal((error as { code?: string }).code, "invalid_metadata_block");
      return true;
    });
  });
}
