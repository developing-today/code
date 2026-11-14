/**
 * Generate manifest.json mapping logical names to hashed filenames.
 * This is read by the Rust server to inject correct asset URLs into HTML.
 *
 * Handles both pre-hashed files (Bun JS output) and unhashed files
 * (Tailwind CLI CSS output) by computing SHA256 hashes as needed.
 *
 * Re-runnable on an already-built dist (#6969): the CSS step adopts an
 * existing contract-valid hashed stylesheet instead of deleting it, so
 * `bun run build:manifest` twice leaves dist styled and the manifest
 * entry intact.
 */

import { createHash } from "node:crypto";
import { readFileSync, readdirSync, renameSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const distDir = "dist";
const manifest: Record<string, string> = {};

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
    try {
      unlinkSync(join(distDir, `${stale.name}.map`));
    } catch {}
  }
}

// Stylesheet names this script owns follow the emitted-name contract in
// pkgs/id/src/web/assets.rs (#6779): `styles.` + exactly 8 lowercase hex
// chars (a truncated SHA-256) + `.css`. Names outside the contract
// (e.g. styles.darkmode.css) are not build outputs and are never
// deleted or adopted here.
const hashedStylesRe = /^styles\.[0-9a-f]{8}\.css$/;
const cssFiles = readdirSync(distDir);
const hashedStyles = cssFiles
  .filter((file) => hashedStylesRe.test(file))
  .map((name) => ({ name, mtime: statSync(join(distDir, name)).mtimeMs }))
  .sort((a, b) => b.mtime - a.mtime);

if (cssFiles.includes("styles.css")) {
  // Tailwind CLI outputs "styles.css" (unhashed) — compute hash and rename
  const content = readFileSync(join(distDir, "styles.css"));
  const hash = createHash("sha256").update(content).digest("hex").slice(0, 8);
  const hashedName = `styles.${hash}.css`;
  renameSync(join(distDir, "styles.css"), join(distDir, hashedName));
  manifest["styles.css"] = hashedName;
  // Rotation: any other hashed stylesheet is stale now.
  for (const stale of hashedStyles) {
    if (stale.name !== hashedName) {
      unlinkSync(join(distDir, stale.name));
    }
  }
} else if (hashedStyles.length > 0) {
  // Re-run on an already-built dist: there is no unhashed styles.css to
  // rename, so adopt the newest contract-valid hashed stylesheet (the
  // JS-bundle newest-wins rule above) instead of deleting the only
  // stylesheet and dropping its manifest entry (#6969).
  manifest["styles.css"] = hashedStyles[0].name;
  for (const stale of hashedStyles.slice(1)) {
    unlinkSync(join(distDir, stale.name));
  }
}

writeFileSync(join(distDir, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log("  manifest.json:", manifest);
