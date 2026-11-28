// Build-time half of generate-client.sh: strip the types from the real
// generated client (client.ts) into client.browser.js, which a browser can
// import directly. Uses Node's own strip-only transform — the same
// erasable-syntax contract the client is written to (docs/story.md,
// "Runtime portability") — so the browser artifact is the generated client
// verbatim minus annotations, never a hand-maintained copy.
//
// Usage: node strip-client.mjs client.ts client.browser.js

import { stripTypeScriptTypes } from "node:module";
import { readFileSync, writeFileSync } from "node:fs";

const [input, output] = process.argv.slice(2);
if (!input || !output) {
  console.error("usage: node strip-client.mjs <client.ts> <client.browser.js>");
  process.exit(2);
}
const src = readFileSync(input, "utf8");
const stripped = stripTypeScriptTypes(src);
writeFileSync(
  output,
  "// Generated from client.ts by strip-client.mjs (strip-only). Do not edit;\n" +
    "// regenerate with generate-client.sh.\n" +
    stripped,
);
console.log(`stripped ${input} -> ${output}`);
