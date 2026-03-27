/**
 * Generate manifest.json mapping logical names to hashed filenames.
 * This is read by the Rust server to inject correct asset URLs into HTML.
 *
 * Handles both pre-hashed files (Bun JS output) and unhashed files
 * (Tailwind CLI CSS output) by computing SHA256 hashes as needed.
 */

import { createHash } from "node:crypto";
import {
  readFileSync,
  readdirSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";

const distDir = "dist";
const manifest: Record<string, string> = {};

// Remove stale hashed CSS files before processing
for (const file of readdirSync(distDir)) {
  if (file.match(/^styles\.[a-f0-9]+\.css$/)) {
    unlinkSync(join(distDir, file));
  }
}

for (const file of readdirSync(distDir)) {
  // Bun outputs "main.{base36hash}.js" (already hashed)
  const jsMatch = file.match(/^(main)\.([a-z0-9]+)\.js$/);

  // Tailwind CLI outputs "styles.css" (unhashed) — compute hash and rename
  const cssExact = file === "styles.css";

  if (jsMatch) {
    manifest["main.js"] = file;
  } else if (cssExact) {
    const content = readFileSync(join(distDir, file));
    const hash = createHash("sha256").update(content).digest("hex").slice(0, 8);
    const hashedName = `styles.${hash}.css`;
    renameSync(join(distDir, file), join(distDir, hashedName));
    manifest["styles.css"] = hashedName;
  }
}

writeFileSync(join(distDir, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log("  manifest.json:", manifest);
