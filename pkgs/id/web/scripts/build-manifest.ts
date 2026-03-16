/**
 * Generate manifest.json mapping logical names to hashed filenames.
 * This is read by the Rust server to inject correct asset URLs into HTML.
 */

import { readdirSync, writeFileSync } from "fs";
import { join } from "path";

const distDir = "dist";
const manifest: Record<string, string> = {};

for (const file of readdirSync(distDir)) {
  // Match files like "main.abc12xyz.js" or "styles.abc12345.css"
  // Bun uses base36 hashes (alphanumeric), CSS build uses hex hashes
  const jsMatch = file.match(/^(main)\.([a-z0-9]+)\.js$/);
  const cssMatch = file.match(/^(styles)\.([a-f0-9]+)\.css$/);
  
  if (jsMatch) {
    manifest["main.js"] = file;
  } else if (cssMatch) {
    manifest["styles.css"] = file;
  }
}

writeFileSync(join(distDir, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log("  manifest.json:", manifest);
