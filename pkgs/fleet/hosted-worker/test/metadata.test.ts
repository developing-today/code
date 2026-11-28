// Metadata block parser tests (#16075). Oracle: the issue's
// acceptance — fence parsing recognizes a closing delimiter line,
// not backticks inside JSON string contents; a render/parse
// round-trip preserves string fields containing triple backticks;
// missing markers, malformed JSON and genuinely invalid delimiters
// still fail closed (400 invalid_metadata_block).

import assert from "node:assert/strict";
import { test } from "node:test";
import { FailClosed } from "../src/errors.ts";
import { METADATA_END, METADATA_START, parseMetadataBlock, renderMetadataBlock } from "../src/metadata.ts";

function assertInvalidBlock(body: string): void {
  assert.throws(
    () => parseMetadataBlock(body),
    (err: unknown) => err instanceof FailClosed && err.code === "invalid_metadata_block",
  );
}

test("round-trip preserves string fields containing triple backticks", () => {
  const block: Record<string, unknown> = {
    pm_meta: 1,
    direct_parent: null,
    links: [
      { title: "Document ```json examples", url: "https://example.com/a```b" },
      { title: "```", url: "https://example.com/" },
    ],
    lineage: { design: "see ``` fences", roadmap_item: null, priority: null },
    note: "value ends with a fence ```",
  };
  const body = `Intro text\n\n${renderMetadataBlock(block)}\n\nTrailing notes`;
  const parsed = parseMetadataBlock(body);
  assert.deepEqual(parsed.block, block);
  assert.equal(parsed.before, "Intro text\n\n");
  assert.equal(parsed.after, "\n\nTrailing notes");
});

test("backticks at a payload line start inside a string do not end the fence", () => {
  // Caller-formatted payload: the note string opens on one line and
  // its continuation line begins with backticks. The closing fence
  // is the delimiter line after the payload, so the full payload is
  // handed to JSON.parse (which then judges the payload itself).
  const payload = ['{ "pm_meta": 1,', '  "note": "a', "```", 'b" }'].join("\n");
  const body = `${METADATA_START}\n\`\`\`json\n${payload}\n\`\`\`\n${METADATA_END}`;
  // The bare-backtick line is the delimiter line, so the text after
  // it is non-whitespace outside the fence: the block fails closed
  // rather than the parser fishing past it for a later fence and
  // accepting a payload the caller did not delimit.
  assertInvalidBlock(body);
});

test("genuinely invalid delimiters and payloads still fail closed", () => {
  // No fence at all between the markers.
  assertInvalidBlock(`${METADATA_START}\n{ "pm_meta": 1 }\n${METADATA_END}`);
  // Opening fence without a closing delimiter line.
  assertInvalidBlock(`${METADATA_START}\n\`\`\`json\n{ "pm_meta": 1 }\n${METADATA_END}`);
  // Non-whitespace content after the closing fence.
  assertInvalidBlock(`${METADATA_START}\n\`\`\`json\n{ "pm_meta": 1 }\n\`\`\`\ntrailing\n${METADATA_END}`);
  // Malformed JSON inside a well-formed fence.
  assertInvalidBlock(`${METADATA_START}\n\`\`\`json\n{ pm_meta: 1 }\n\`\`\`\n${METADATA_END}`);
  // Well-formed JSON that is not a pm_meta: 1 object.
  assertInvalidBlock(`${METADATA_START}\n\`\`\`json\n{ "pm_meta": 2 }\n\`\`\`\n${METADATA_END}`);
  // End marker without a start marker.
  assertInvalidBlock(`body\n${METADATA_END}`);
});

test("a body with no markers parses to a null block", () => {
  const parsed = parseMetadataBlock("plain body, no metadata");
  assert.equal(parsed.block, null);
  assert.equal(parsed.before, "plain body, no metadata");
  assert.equal(parsed.after, "");
});
