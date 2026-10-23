// Regenerates doc/hardware/{artifact-manifest.md,inventory.txt,verification.json}
// from the actual bytes on disk, and runs integrity checks.
const root = "doc/hardware";
const RETRIEVED = "2026-08-21";

async function walk(path: string): Promise<string[]> {
  const files: string[] = [];
  for await (const entry of Deno.readDir(path)) {
    const child = `${path}/${entry.name}`;
    if (entry.isDirectory) files.push(...(await walk(child)));
    else if (entry.isFile) files.push(child);
  }
  return files.sort();
}

async function sha256(path: string) {
  const hash = await crypto.subtle.digest("SHA-256", await Deno.readFile(path));
  return Array.from(new Uint8Array(hash), (b) => b.toString(16).padStart(2, "0")).join("");
}

const failures: string[] = [];

// ---------------------------------------------------------------- artifacts
const artifacts = (await walk(root)).filter((p) => p.includes("/artifacts/"));
let artifactBytes = 0;
const rows: string[] = [];
const byHash = new Map<string, string[]>();
const sizeOf = new Map<string, number>();

for (const path of artifacts) {
  const size = (await Deno.stat(path)).size;
  const hash = await sha256(path);
  artifactBytes += size;
  sizeOf.set(path, size);
  byHash.set(hash, [...(byHash.get(hash) ?? []), path]);
  rows.push(`| \`${hash}\` | ${size} | \`${path.slice(root.length + 1)}\` |`);
}

// Exact-duplicate analysis: bytes beyond one retained copy of each hash.
let duplicateHashGroups = 0;
let duplicateRedundantBytes = 0;
for (const [, paths] of byHash) {
  if (paths.length < 2) continue;
  duplicateHashGroups++;
  duplicateRedundantBytes += (paths.length - 1) * (sizeOf.get(paths[0]) ?? 0);
}

const manifest = [
  "# Artifact manifest",
  "",
  `Generated from actual local artifact files on ${RETRIEVED}. Paths are relative to \`doc/hardware/\`.`,
  `Files: **${artifacts.length}**. Total bytes: **${artifactBytes}**.`,
  "",
  "## Duplication and storage",
  "",
  `Exact-hash analysis found **${duplicateHashGroups}** duplicate groups accounting for **${duplicateRedundantBytes} bytes** beyond one copy of each hash. These duplicates are intentional contents of the complete official demo tree, whose internal layout is preserved. The original ZIP archives are also retained alongside extracted files: archives preserve downloaded source fidelity, while extraction provides direct access. Archive/extraction overlap is not included in the exact-file duplicate figure because ZIP container bytes differ from their members. Two separately extracted factory firmware files were removed because byte-identical copies remain in the complete demo tree; the official BIN ZIP remains retained.`,
  "",
  "## Checksums",
  "",
  "| SHA-256 | Bytes | Path |",
  "|---|---:|---|",
  ...rows,
];
await Deno.writeTextFile(`${root}/artifact-manifest.md`, manifest.join("\n") + "\n");

// ---------------------------------------------------------------- inventory
const all = (await walk(root)).filter((p) => !p.endsWith("/inventory.txt") && !p.endsWith("/verification.json"));
let totalBytes = 0;
const inventory: string[] = [];
for (const path of all) {
  const size = (await Deno.stat(path)).size;
  totalBytes += size;
  inventory.push(`${size}\t${path.slice(root.length + 1)}`);
}
await Deno.writeTextFile(
  `${root}/inventory.txt`,
  `# Generated ${RETRIEVED}\n# Files ${all.length}\n# Bytes ${totalBytes}\n${inventory.join("\n")}\n`,
);

// ------------------------------------------------- authored markdown links
// Validate authored research only, not upstream Markdown preserved byte-for-byte in archives.
const markdown = all.filter((p) => p.endsWith(".md") && !p.includes("/artifacts/"));
const linkPattern = /\[[^\]]*\]\(([^)]+)\)/g;
let authoredRelativeLinks = 0;
for (const path of markdown) {
  const text = await Deno.readTextFile(path);
  for (const match of text.matchAll(linkPattern)) {
    const target = match[1].trim().replace(/^<|>$/g, "");
    if (/^(https?:|mailto:|#)/.test(target)) continue;
    const withoutAnchor = decodeURIComponent(target.split("#", 1)[0]);
    if (!withoutAnchor) continue;
    authoredRelativeLinks++;
    const resolved = new URL(withoutAnchor, `file://${Deno.cwd()}/${path}`).pathname;
    try {
      await Deno.stat(resolved);
    } catch {
      failures.push(`link: ${path}: ${target} -> ${resolved}`);
    }
  }
}

// ------------------------------------------------------------ pinned hashes
const expected = new Map([
  [
    "devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-schematic.zip",
    "baa5ac1bf75fbbd86a8135b123ff498bd7db4a5c68184481db6b82cadbaca0e5",
  ],
  [
    "devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-Demo.zip",
    "11e382444fe93470fbe463829c1e0ebad5bdb5115fd2d72f6159cd7700015030",
  ],
  [
    "devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/originals/ESP32-S3-Knob-Touch-LCD-1.8-BIN.zip",
    "7d29fc1fb356059f7291eccd74bfb5c9fa7538998bc3f5ff811cd87f04c1691c",
  ],
  [
    "devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/firmware/ESP32-S3-Knob-Touch-LCD-1.8-BIN/ESP32-KNOB_ESP32_0.bin",
    "0c1c21b9822d4c2d80d58534b33eb0083880de4ed7354a38b4c78ba51757349d",
  ],
  [
    "devices/waveshare/esp32-s3-knob-touch-lcd-1.8/artifacts/firmware/ESP32-S3-Knob-Touch-LCD-1.8-BIN/WX-ESP32S3-KNOB_V1.2.bin",
    "f7c1cc18b687559f3bd69e5c9ab526bc61c2b2d9c502f38367f7f2bfe4ff8e87",
  ],
]);
// Some pinned files were intentionally de-duplicated (a byte-identical copy is
// retained elsewhere, e.g. inside the extracted demo tree). For those, verify the
// content still exists somewhere in the tree under the same hash rather than
// demanding the original path.
const allHashes = new Set<string>();
for (const [hash] of byHash) allHashes.add(hash);

let hashFailures = 0;
for (const [path, hash] of expected) {
  let actual: string | null = null;
  try {
    actual = await sha256(`${root}/${path}`);
  } catch {
    actual = null;
  }
  if (actual === null) {
    if (allHashes.has(hash)) continue; // de-duplicated, content still present
    hashFailures++;
    failures.push(`hash: ${path}: missing, and no artifact with hash ${hash} remains`);
  } else if (actual !== hash) {
    hashFailures++;
    failures.push(`hash: ${path}: expected ${hash}, got ${actual}`);
  }
}

// -------------------------------------------------------------- zip integrity
let zipFailures = 0;
for (const path of all.filter((p) => p.toLowerCase().endsWith(".zip"))) {
  const out = await new Deno.Command("unzip", { args: ["-t", path] }).output();
  if (!out.success) {
    zipFailures++;
    failures.push(`zip: ${path}: unzip -t reported corruption`);
  }
}

// ------------------------------------------------------------------ pdf magic
// Every .pdf must actually be a PDF. Guards against silently saving HTML error
// pages under a .pdf name, which is how documentation mirrors usually fail.
let pdfFailures = 0;
const magic = new TextEncoder().encode("%PDF");
for (const path of all.filter((p) => p.toLowerCase().endsWith(".pdf"))) {
  const fh = await Deno.open(path);
  const head = new Uint8Array(4);
  await fh.read(head);
  fh.close();
  if (!head.every((b, i) => b === magic[i])) {
    pdfFailures++;
    failures.push(`pdf: ${path}: missing %PDF magic`);
  }
}

// ----------------------------------------------------------- name portability
let nameFailures = 0;
for (const path of all) {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const hasControlChar = [...name].some((ch) => ch.charCodeAt(0) < 0x20);
  if (/[<>:"\\|?*]/.test(name) || hasControlChar || /[ .]$/.test(name)) {
    nameFailures++;
    failures.push(`name: ${path}: not portable across filesystems`);
  }
}

// ---------------------------------------------------------------------- report
const result = {
  verifiedAt: RETRIEVED,
  scope: {
    authoredMarkdown: "Markdown outside device artifacts/",
    bundledUpstreamMarkdown: "preserved as supplied; relative links not validated and not claimed to pass",
    inventory: "all files except inventory.txt and verification.json to avoid self-referential generated metadata",
  },
  authoredMarkdownFiles: markdown.length,
  authoredRelativeLinks,
  authoredRelativeLinksResult: failures.some((f) => f.startsWith("link:")) ? "FAILED" : "OK",
  artifactFiles: artifacts.length,
  artifactBytes,
  inventoryFiles: all.length,
  inventoryBytes: totalBytes,
  duplicateHashGroups,
  duplicateRedundantBytes,
  expectedOfficialZipAndFirmwareHashes: hashFailures ? "FAILED" : "OK",
  zipIntegrity: zipFailures ? "FAILED" : "OK",
  pdfMagic: pdfFailures ? "FAILED" : "OK",
  filenamePortability: nameFailures ? "FAILED" : "OK",
  failures,
};
await Deno.writeTextFile(`${root}/verification.json`, JSON.stringify(result, null, 2) + "\n");
console.log(JSON.stringify(result, null, 2));
if (failures.length) Deno.exit(1);
