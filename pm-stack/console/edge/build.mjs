// Puts the app's files where the Worker serves them from: dist/console/
// (the page, its script and style, its service worker) and dist/icons/
// (the install icons, which live with the dashboard).
import { cpSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

if (process.argv.length > 2) {
  console.error("usage: node build.mjs   (it takes no arguments)");
  process.exit(2);
}
const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "dist");
rmSync(dist, { recursive: true, force: true });
mkdirSync(join(dist, "console"), { recursive: true });
mkdirSync(join(dist, "icons"), { recursive: true });
for (const name of ["index.html", "app.js", "app.css", "icon.svg", "sw.js"]) {
  cpSync(join(here, "..", "public", name), join(dist, "console", name));
}
for (const name of ["icon-192.png", "icon-512.png", "icon-maskable-512.png", "apple-touch-icon.png"]) {
  cpSync(join(here, "..", "..", "aggregator", "src", "assets", "icons", name), join(dist, "icons", name));
}
console.log(`built ${dist}`);
