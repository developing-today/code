/**
 * Build CSS with content hashing for cache busting.
 * Concatenates all CSS files and outputs with a hash in the filename.
 */

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, readdirSync, unlinkSync } from "node:fs";
import { join } from "node:path";

const cssFiles = [
  "node_modules/prosemirror-view/style/prosemirror.css",
  "node_modules/prosemirror-menu/style/menu.css",
  "node_modules/prosemirror-example-setup/style/style.css",
  "styles/terminal.css",
  "styles/themes.css",
  "styles/editor.css",
];

// Read and concatenate all CSS
const css = cssFiles.map((file) => readFileSync(file, "utf-8")).join("\n");

// Generate content hash (first 8 chars of SHA256)
const hash = createHash("sha256").update(css).digest("hex").slice(0, 8);
const filename = `styles.${hash}.css`;

// Remove old hashed CSS files
const distDir = "dist";
for (const file of readdirSync(distDir)) {
  if (file.startsWith("styles.") && file.endsWith(".css")) {
    unlinkSync(join(distDir, file));
  }
}

// Write new hashed CSS
writeFileSync(join(distDir, filename), css);
console.log(`  ${filename}  ${(css.length / 1024).toFixed(1)} KB`);
