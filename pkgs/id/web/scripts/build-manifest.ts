/**
 * Generate manifest.json mapping logical names to hashed filenames.
 * This is read by the Rust server to inject correct asset URLs into HTML.
 *
 * Handles both pre-hashed files (Bun JS output) and unhashed files
 * (Tailwind CLI CSS output) by computing SHA256 hashes as needed.
 */

import { createHash } from "node:crypto";
import { readFileSync, readdirSync, renameSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const distDir = "dist";
const manifest: Record<string, string> = {};

// Remove stale hashed CSS files before processing
for (const file of readdirSync(distDir)) {
  if (file.match(/^styles\.[a-f0-9]+\.css$/)) {
    unlinkSync(join(distDir, file));
  }
}

// Find the newest JS bundle by mtime and remove stale ones
const jsFiles: { name: string; mtime: number }[] = [];
for (const file of readdirSync(distDir)) {
  if (file.match(/^main\.[a-z0-9]+\.js$/)) {
    const { mtimeMs } = statSync(join(distDir, file));
    jsFiles.push({ name: file, mtime: mtimeMs });
  }
}
if (jsFiles.length > 0) {
  jsFiles.sort((a, b) => b.mtime - a.mtime);
  manifest["main.js"] = jsFiles[0].name;
  // Remove stale JS bundles and their source maps
  for (const stale of jsFiles.slice(1)) {
    unlinkSync(join(distDir, stale.name));
    try { unlinkSync(join(distDir, `${stale.name}.map`)); } catch {}
  }
}

for (const file of readdirSync(distDir)) {
  // Tailwind CLI outputs "styles.css" (unhashed) — compute hash and rename
  const cssExact = file === "styles.css";

  if (cssExact) {
    const content = readFileSync(join(distDir, file));
    const hash = createHash("sha256").update(content).digest("hex").slice(0, 8);
    const hashedName = `styles.${hash}.css`;
    renameSync(join(distDir, file), join(distDir, hashedName));
    manifest["styles.css"] = hashedName;
  }
}

writeFileSync(join(distDir, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log("  manifest.json:", manifest);
