#!/usr/bin/env bun
// Generate the PWA icons (#3676 / #3315) into src/assets/icons/.
// A tiny hand-rolled PNG encoder (zlib + CRC32) drawing a simple
// alert glyph — amber disc, dark exclamation — on the dashboard's
// background color. Re-run after changing the art; the PNGs are
// committed, this script is the source of truth for them.
import { deflateSync } from "node:zlib";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const OUT = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "assets", "icons");

const crcTable = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buf: Uint8Array): number {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, data.length);
  for (let i = 0; i < 4; i++) out[4 + i] = type.charCodeAt(i);
  out.set(data, 8);
  view.setUint32(8 + data.length, crc32(out.subarray(4, 8 + data.length)));
  return out;
}

function encodePng(size: number, rgba: Uint8Array): Uint8Array {
  const raw = new Uint8Array(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // filter: none
    raw.set(rgba.subarray(y * size * 4, (y + 1) * size * 4), y * (size * 4 + 1) + 1);
  }
  const ihdr = new Uint8Array(13);
  const view = new DataView(ihdr.buffer);
  view.setUint32(0, size);
  view.setUint32(4, size);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  const parts = [
    new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", new Uint8Array(0)),
  ];
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

const BG: [number, number, number, number] = [14, 17, 23, 255]; // #0e1117
const AMBER: [number, number, number, number] = [210, 153, 34, 255]; // #d29922
const DARK: [number, number, number, number] = [14, 17, 23, 255];

function draw(size: number, glyphScale: number): Uint8Array {
  const px = new Uint8Array(size * size * 4);
  const set = (x: number, y: number, c: [number, number, number, number]) => {
    if (x < 0 || y < 0 || x >= size || y >= size) return;
    const i = (y * size + x) * 4;
    px[i] = c[0];
    px[i + 1] = c[1];
    px[i + 2] = c[2];
    px[i + 3] = c[3];
  };
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) set(x, y, BG);
  const cx = size / 2;
  const cy = size / 2;
  const r = (size / 2) * glyphScale;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const d = Math.hypot(x - cx, y - cy);
      if (d <= r) set(x, y, AMBER);
    }
  }
  // Exclamation: bar from 32%..58% of the disc diameter, dot at 72%.
  const barHalf = size * 0.035 * glyphScale + 1;
  const barTop = cy - r * 0.55;
  const barBottom = cy + r * 0.18;
  const dotR = barHalf * 1.15;
  const dotCy = cy + r * 0.52;
  for (let y = Math.floor(barTop); y <= barBottom; y++) {
    for (let x = Math.floor(cx - barHalf); x <= cx + barHalf; x++) set(x, y, DARK);
  }
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (Math.hypot(x - cx, y - dotCy) <= dotR) set(x, y, DARK);
    }
  }
  return px;
}

await mkdir(OUT, { recursive: true });
const files: [string, number, number][] = [
  ["icon-192.png", 192, 0.92],
  ["icon-512.png", 512, 0.92],
  ["icon-maskable-512.png", 512, 0.72], // padded safe zone for maskable
  ["apple-touch-icon.png", 180, 0.92],
];
for (const [name, size, scale] of files) {
  await writeFile(join(OUT, name), encodePng(size, draw(size, scale)));
  console.log(`wrote ${name} (${size}x${size})`);
}
