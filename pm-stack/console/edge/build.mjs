// Puts the app's files where the Worker serves them from: dist/console/
// (the page and every file public/files.json names: its scripts, style
// sheets, typefaces and service worker) and dist/icons/ (the install
// icons, which live with the dashboard).
//
// The list of files is not kept here: public/files.json is the one
// place a file of the app is named.
import { cpSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const usage = "usage: node build.mjs [--out <directory>]   (the directory is emptied first; without one it is ./dist)";
const args = process.argv.slice(2);
const here = dirname(fileURLToPath(import.meta.url));
let dist = join(here, "dist");
if (args.length === 2 && args[0] === "--out" && args[1] && !args[1].startsWith("-")) dist = resolve(args[1]);
else if (args.length > 0) {
  console.error(usage);
  process.exit(2);
}
const from = join(here, "..", "public");
const app = JSON.parse(readFileSync(join(from, "files.json"), "utf8"));
rmSync(dist, { recursive: true, force: true });
mkdirSync(join(dist, "console"), { recursive: true });
mkdirSync(join(dist, "icons"), { recursive: true });
for (const name of [app.page, ...app.files.map((f) => f.path)]) {
  const to = join(dist, "console", name);
  mkdirSync(dirname(to), { recursive: true });
  cpSync(join(from, name), to);
}
for (const name of ["icon-192.png", "icon-512.png", "icon-maskable-512.png", "apple-touch-icon.png"]) {
  cpSync(join(here, "..", "..", "aggregator", "src", "assets", "icons", name), join(dist, "icons", name));
}
console.log(`built ${dist}: the page and ${app.files.length} files`);
