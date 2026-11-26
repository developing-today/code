/** Safe browser presenter for the world screen wire model. */

export type ScreenSurface = {
  kind: "screen";
  version: number;
  width: number;
  height: number;
  cells: ScreenCell[];
};

export type ScreenCell = {
  symbol: string;
  fg: number;
  bg: number;
  bold: boolean;
  columns: number;
  /** Style token name from the wire model, when the frame carries one. */
  token?: string | null;
};

export type ScreenPatch = {
  version: number;
  width: number;
  height: number;
  runs: Array<{ start: number; cells: ScreenCell[] }>;
};

/** Apply a bounded, ordered row-major patch without mutating its base. */
export function applyScreenPatch(previous: ScreenSurface, patch: ScreenPatch): ScreenSurface | null {
  if (
    !previous ||
    !patch ||
    patch.version !== previous.version ||
    patch.width !== previous.width ||
    patch.height !== previous.height ||
    !Array.isArray(patch.runs) ||
    previous.cells.length !== previous.width * previous.height
  ) {
    return null;
  }
  const cells = previous.cells.slice();
  let coveredUntil = 0;
  for (const run of patch.runs) {
    if (!run || !Number.isInteger(run.start) || run.start < coveredUntil || !Array.isArray(run.cells)) return null;
    const end = run.start + run.cells.length;
    if (!Number.isSafeInteger(end) || end > cells.length) return null;
    for (let index = 0; index < run.cells.length; index++) cells[run.start + index] = run.cells[index];
    coveredUntil = end;
  }
  return { ...previous, cells };
}

const MAX_SCREEN_WIDTH = 120;
const MAX_SCREEN_HEIGHT = 50;
const MAX_SCREEN_CELLS = MAX_SCREEN_WIDTH * MAX_SCREEN_HEIGHT;

// ⚡ Bolt optimization: renderWorldSurface calls xtermColor twice per cell —
// up to 12,000 calls for a full 120x50 frame, once per incoming screen patch.
// The 256-entry xterm palette is fixed, so resolve it once at module load
// instead of rebuilding the 16-entry basic-colour array and formatting strings
// on every call. Index 0 means "no explicit colour" (null), as before.
const XTERM_PALETTE: ReadonlyArray<string | null> = (() => {
  const basic = [
    "#000000",
    "#800000",
    "#008000",
    "#808000",
    "#000080",
    "#800080",
    "#008080",
    "#c0c0c0",
    "#808080",
    "#ff0000",
    "#00ff00",
    "#ffff00",
    "#0000ff",
    "#ff00ff",
    "#00ffff",
    "#ffffff",
  ];
  const palette: Array<string | null> = new Array(256).fill(null);
  for (let index = 1; index < 16; index++) palette[index] = basic[index] ?? null;
  for (let index = 16; index <= 231; index++) {
    const value = index - 16;
    const level = (component: number) => (component === 0 ? 0 : 55 + component * 40);
    const r = level(Math.floor(value / 36));
    const g = level(Math.floor((value % 36) / 6));
    const b = level(value % 6);
    palette[index] = `#${[r, g, b].map((part) => part.toString(16).padStart(2, "0")).join("")}`;
  }
  for (let index = 232; index < 256; index++) {
    const gray = 8 + (index - 232) * 10;
    palette[index] = `#${gray.toString(16).padStart(2, "0").repeat(3)}`;
  }
  return palette;
})();

function xtermColor(index: number): string | null {
  if (!Number.isInteger(index) || index < 0 || index > 255) return null;
  return XTERM_PALETTE[index] ?? null;
}

/**
 * True when every code point in `symbol` is a printable, non-control char.
 * Iterates code units instead of allocating an array via Array.from(): control
 * characters (C0, DEL, C1) all live in the BMP, so a surrogate half always
 * passes here exactly as its code point did before.
 */
function isPrintableSymbol(symbol: string): boolean {
  for (let i = 0; i < symbol.length; i++) {
    const code = symbol.charCodeAt(i);
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) return false;
  }
  return true;
}

/**
 * Flatten an ascending, non-overlapping `[low, high]` range table into
 * `[low0, high0, low1, high1, ...]` so `rangeContains` can binary-search
 * it without an array-of-arrays indirection per comparison.
 */
function flattenRanges(ranges: ReadonlyArray<readonly [number, number]>): Int32Array {
  const flat = new Int32Array(ranges.length * 2);
  for (let index = 0; index < ranges.length; index++) {
    const [low, high] = ranges[index];
    flat[index * 2] = low;
    flat[index * 2 + 1] = high;
  }
  return flat;
}

/**
 * Membership test over a flattened ascending range table, in O(log n).
 *
 * ⚡ Bolt optimization: these tables are ~80–180 entries and were scanned
 * linearly (`ranges.some(...)`) once per non-ASCII cell of every rendered
 * world frame. A 120x50 frame of box-drawing glyphs took ~52 ms of CPU
 * instead of ~1 ms, because each cell paid the full linear scan. Binary
 * search over the flat `Int32Array` cuts the lookup to a handful of
 * integer comparisons with no allocation. `flattenRanges` preserves the
 * table order, so callers must keep the literal tables sorted and
 * disjoint (verified exhaustively over U+0000..U+10FFFF in this change).
 */
function rangeContains(flat: Int32Array, codePoint: number): boolean {
  let low = 0;
  let high = (flat.length >> 1) - 1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    const start = flat[mid * 2];
    const end = flat[mid * 2 + 1];
    if (codePoint < start) high = mid - 1;
    else if (codePoint > end) low = mid + 1;
    else return true;
  }
  return false;
}

// Wide code-point ranges matching Rust unicode-width 0.2's wide table,
// including the astral planes the previous BMP-only regex missed (#761):
// emoji blocks (U+1F300–U+1FAF8 ranges) and CJK Ext. B and above
// (U+20000–U+2FFFD, U+30000–U+3FFFD).
// Must stay sorted ascending and non-overlapping: `rangeContains` assumes it.
const WIDE_RANGES: ReadonlyArray<readonly [number, number]> = [
  [0x1100, 0x115f],
  [0x17a4, 0x17a4],
  // BMP default-emoji-presentation ranges (the BMP part of
  // TEXT_PRESENTATION_RANGES below): wide alone under unicode-width
  // 0.2.2, narrowing to 1 with U+FE0E (#1581).
  [0x231a, 0x231b],
  [0x2329, 0x232a],
  [0x23e9, 0x23ec],
  [0x23f0, 0x23f0],
  [0x23f3, 0x23f3],
  [0x25fd, 0x25fe],
  [0x2614, 0x2615],
  [0x2630, 0x2637],
  [0x2648, 0x2653],
  [0x267f, 0x267f],
  [0x268a, 0x268f],
  [0x2693, 0x2693],
  [0x26a1, 0x26a1],
  [0x26aa, 0x26ab],
  [0x26bd, 0x26be],
  [0x26c4, 0x26c5],
  [0x26ce, 0x26ce],
  [0x26d4, 0x26d4],
  [0x26ea, 0x26ea],
  [0x26f2, 0x26f3],
  [0x26f5, 0x26f5],
  [0x26fa, 0x26fa],
  [0x26fd, 0x26fd],
  [0x2705, 0x2705],
  [0x270a, 0x270b],
  [0x2728, 0x2728],
  [0x274c, 0x274c],
  [0x274e, 0x274e],
  [0x2753, 0x2755],
  [0x2757, 0x2757],
  [0x2795, 0x2797],
  [0x27b0, 0x27b0],
  [0x27bf, 0x27bf],
  [0x2b1b, 0x2b1c],
  [0x2b50, 0x2b50],
  [0x2b55, 0x2b55],
  [0x2e80, 0x2e99],
  [0x2e9b, 0x2ef3],
  [0x2f00, 0x2fd5],
  [0x2ff0, 0x303e],
  [0x3041, 0x3096],
  [0x3099, 0x30ff],
  [0x3105, 0x312f],
  [0x3131, 0x318e],
  [0x3190, 0x31e5],
  [0x31ef, 0x321e],
  [0x3220, 0x3247],
  [0x3250, 0xa48c],
  [0xa490, 0xa4c6],
  [0xa960, 0xa97c],
  [0xac00, 0xd7a3],
  [0xf900, 0xfaff],
  [0xfe10, 0xfe19],
  [0xfe30, 0xfe52],
  [0xfe54, 0xfe66],
  [0xfe68, 0xfe6b],
  [0xff01, 0xff60],
  [0xffe0, 0xffe6],
  [0x16fe0, 0x16fe4],
  [0x16ff0, 0x16ff6],
  [0x17000, 0x18cd5],
  [0x18cff, 0x18d1e],
  [0x18d80, 0x18df2],
  [0x1aff0, 0x1aff3],
  [0x1aff5, 0x1affb],
  [0x1affd, 0x1affe],
  [0x1b000, 0x1b122],
  [0x1b132, 0x1b132],
  [0x1b150, 0x1b152],
  [0x1b155, 0x1b155],
  [0x1b164, 0x1b167],
  [0x1b170, 0x1b2fb],
  [0x1d300, 0x1d356],
  [0x1d360, 0x1d376],
  [0x1f004, 0x1f004],
  [0x1f0cf, 0x1f0cf],
  [0x1f18e, 0x1f18e],
  [0x1f191, 0x1f19a],
  [0x1f200, 0x1f202],
  [0x1f210, 0x1f23b],
  [0x1f240, 0x1f248],
  [0x1f250, 0x1f251],
  [0x1f260, 0x1f265],
  [0x1f300, 0x1f320],
  [0x1f32d, 0x1f335],
  [0x1f337, 0x1f37c],
  [0x1f37e, 0x1f393],
  [0x1f3a0, 0x1f3ca],
  [0x1f3cf, 0x1f3d3],
  [0x1f3e0, 0x1f3f0],
  [0x1f3f4, 0x1f3f4],
  [0x1f3f8, 0x1f43e],
  [0x1f440, 0x1f440],
  [0x1f442, 0x1f4fc],
  [0x1f4ff, 0x1f53d],
  [0x1f54b, 0x1f54e],
  [0x1f550, 0x1f567],
  [0x1f57a, 0x1f57a],
  [0x1f595, 0x1f596],
  [0x1f5a4, 0x1f5a4],
  [0x1f5fb, 0x1f64f],
  [0x1f680, 0x1f6c5],
  [0x1f6cc, 0x1f6cc],
  [0x1f6d0, 0x1f6d2],
  [0x1f6d5, 0x1f6d8],
  [0x1f6dc, 0x1f6df],
  [0x1f6eb, 0x1f6ec],
  [0x1f6f4, 0x1f6fc],
  [0x1f7e0, 0x1f7eb],
  [0x1f7f0, 0x1f7f0],
  [0x1f90c, 0x1f93a],
  [0x1f93c, 0x1f945],
  [0x1f947, 0x1f9ff],
  [0x1fa70, 0x1fa7c],
  [0x1fa80, 0x1fa8a],
  [0x1fa8e, 0x1fac6],
  [0x1fac8, 0x1fac8],
  [0x1facd, 0x1fadc],
  [0x1fadf, 0x1faea],
  [0x1faef, 0x1faf8],
  [0x20000, 0x2fffd],
  [0x30000, 0x3fffd],
];

const WIDE_CODE_POINTS = flattenRanges(WIDE_RANGES);

function isWideCodePoint(codePoint: number): boolean {
  return rangeContains(WIDE_CODE_POINTS, codePoint);
}

// Code points that measure 2 alone but narrow to 1 when immediately
// followed by U+FE0E (variation selector 15, text presentation) under
// unicode-width 0.2 — the crate's default-emoji-presentation set (e.g.
// ⬛ U+2B1B alone is 2 columns, ⬛︎ U+2B1B U+FE0E is 1). Most wide code
// points are NOT in this set: U+1F600 and CJK keep width 2 with U+FE0E.
// Verified exhaustively against unicode-width 0.2.2 (#1581).
const TEXT_PRESENTATION_RANGES: ReadonlyArray<readonly [number, number]> = [
  [0x231a, 0x231b],
  [0x23e9, 0x23ec],
  [0x23f0, 0x23f0],
  [0x23f3, 0x23f3],
  [0x25fd, 0x25fe],
  [0x2614, 0x2615],
  [0x2648, 0x2653],
  [0x267f, 0x267f],
  [0x2693, 0x2693],
  [0x26a1, 0x26a1],
  [0x26aa, 0x26ab],
  [0x26bd, 0x26be],
  [0x26c4, 0x26c5],
  [0x26ce, 0x26ce],
  [0x26d4, 0x26d4],
  [0x26ea, 0x26ea],
  [0x26f2, 0x26f3],
  [0x26f5, 0x26f5],
  [0x26fa, 0x26fa],
  [0x26fd, 0x26fd],
  [0x2705, 0x2705],
  [0x270a, 0x270b],
  [0x2728, 0x2728],
  [0x274c, 0x274c],
  [0x274e, 0x274e],
  [0x2753, 0x2755],
  [0x2757, 0x2757],
  [0x2795, 0x2797],
  [0x27b0, 0x27b0],
  [0x27bf, 0x27bf],
  [0x2b1b, 0x2b1c],
  [0x2b50, 0x2b50],
  [0x2b55, 0x2b55],
  [0x1f004, 0x1f004],
  [0x1f30d, 0x1f30f],
  [0x1f315, 0x1f315],
  [0x1f31c, 0x1f31c],
  [0x1f378, 0x1f378],
  [0x1f393, 0x1f393],
  [0x1f3a7, 0x1f3a7],
  [0x1f3ac, 0x1f3ae],
  [0x1f3c2, 0x1f3c2],
  [0x1f3c4, 0x1f3c4],
  [0x1f3c6, 0x1f3c6],
  [0x1f3ca, 0x1f3ca],
  [0x1f3e0, 0x1f3e0],
  [0x1f3ed, 0x1f3ed],
  [0x1f408, 0x1f408],
  [0x1f415, 0x1f415],
  [0x1f41f, 0x1f41f],
  [0x1f426, 0x1f426],
  [0x1f442, 0x1f442],
  [0x1f446, 0x1f449],
  [0x1f44d, 0x1f44e],
  [0x1f453, 0x1f453],
  [0x1f46a, 0x1f46a],
  [0x1f47d, 0x1f47d],
  [0x1f4a3, 0x1f4a3],
  [0x1f4b0, 0x1f4b0],
  [0x1f4b3, 0x1f4b3],
  [0x1f4bb, 0x1f4bb],
  [0x1f4bf, 0x1f4bf],
  [0x1f4cb, 0x1f4cb],
  [0x1f4da, 0x1f4da],
  [0x1f4df, 0x1f4df],
  [0x1f4e4, 0x1f4e6],
  [0x1f4ea, 0x1f4ed],
  [0x1f4f7, 0x1f4f7],
  [0x1f4f9, 0x1f4fb],
  [0x1f508, 0x1f508],
  [0x1f50d, 0x1f50d],
  [0x1f512, 0x1f513],
  [0x1f550, 0x1f567],
  [0x1f610, 0x1f610],
  [0x1f687, 0x1f687],
  [0x1f68d, 0x1f68d],
  [0x1f691, 0x1f691],
  [0x1f694, 0x1f694],
  [0x1f698, 0x1f698],
  [0x1f6ad, 0x1f6ad],
  [0x1f6b2, 0x1f6b2],
  [0x1f6b9, 0x1f6ba],
  [0x1f6bc, 0x1f6bc],
];

const TEXT_PRESENTATION_CODE_POINTS = flattenRanges(TEXT_PRESENTATION_RANGES);

function isTextPresentationCodePoint(codePoint: number): boolean {
  return rangeContains(TEXT_PRESENTATION_CODE_POINTS, codePoint);
}

// Code points that can start an emoji presentation sequence under
// unicode-width 0.2.2 (its `starts_emoji_presentation_seq` table, decoded
// verbatim from the crate's EMOJI_PRESENTATION_LEAVES bit table): a cluster
// whose U+FE0F immediately follows one of these measures 2; U+FE0F after
// any other base measures 0 in the plain sum (#3265).
const EMOJI_PRESENTATION_SEQ_RANGES: ReadonlyArray<readonly [number, number]> = [
  [0x23, 0x23],
  [0x2a, 0x2a],
  [0x30, 0x39],
  [0xa9, 0xa9],
  [0xae, 0xae],
  [0x203c, 0x203c],
  [0x2049, 0x2049],
  [0x2122, 0x2122],
  [0x2139, 0x2139],
  [0x2194, 0x2199],
  [0x21a9, 0x21aa],
  [0x231a, 0x231b],
  [0x2328, 0x2328],
  [0x23cf, 0x23cf],
  [0x23e9, 0x23f3],
  [0x23f8, 0x23fa],
  [0x24c2, 0x24c2],
  [0x25aa, 0x25ab],
  [0x25b6, 0x25b6],
  [0x25c0, 0x25c0],
  [0x25fb, 0x25fe],
  [0x2600, 0x2604],
  [0x260e, 0x260e],
  [0x2611, 0x2611],
  [0x2614, 0x2615],
  [0x2618, 0x2618],
  [0x261d, 0x261d],
  [0x2620, 0x2620],
  [0x2622, 0x2623],
  [0x2626, 0x2626],
  [0x262a, 0x262a],
  [0x262e, 0x262f],
  [0x2638, 0x263a],
  [0x2640, 0x2640],
  [0x2642, 0x2642],
  [0x2648, 0x2653],
  [0x265f, 0x2660],
  [0x2663, 0x2663],
  [0x2665, 0x2666],
  [0x2668, 0x2668],
  [0x267b, 0x267b],
  [0x267e, 0x267f],
  [0x2692, 0x2697],
  [0x2699, 0x2699],
  [0x269b, 0x269c],
  [0x26a0, 0x26a1],
  [0x26a7, 0x26a7],
  [0x26aa, 0x26ab],
  [0x26b0, 0x26b1],
  [0x26bd, 0x26be],
  [0x26c4, 0x26c5],
  [0x26c8, 0x26c8],
  [0x26ce, 0x26cf],
  [0x26d1, 0x26d1],
  [0x26d3, 0x26d4],
  [0x26e9, 0x26ea],
  [0x26f0, 0x26f5],
  [0x26f7, 0x26fa],
  [0x26fd, 0x26fd],
  [0x2702, 0x2702],
  [0x2705, 0x2705],
  [0x2708, 0x270d],
  [0x270f, 0x270f],
  [0x2712, 0x2712],
  [0x2714, 0x2714],
  [0x2716, 0x2716],
  [0x271d, 0x271d],
  [0x2721, 0x2721],
  [0x2728, 0x2728],
  [0x2733, 0x2734],
  [0x2744, 0x2744],
  [0x2747, 0x2747],
  [0x274c, 0x274c],
  [0x274e, 0x274e],
  [0x2753, 0x2755],
  [0x2757, 0x2757],
  [0x2763, 0x2764],
  [0x2795, 0x2797],
  [0x27a1, 0x27a1],
  [0x27b0, 0x27b0],
  [0x27bf, 0x27bf],
  [0x2934, 0x2935],
  [0x2b05, 0x2b07],
  [0x2b1b, 0x2b1c],
  [0x2b50, 0x2b50],
  [0x2b55, 0x2b55],
  [0x3030, 0x3030],
  [0x303d, 0x303d],
  [0x3297, 0x3297],
  [0x3299, 0x3299],
  [0x1f004, 0x1f004],
  [0x1f170, 0x1f171],
  [0x1f17e, 0x1f17f],
  [0x1f202, 0x1f202],
  [0x1f21a, 0x1f21a],
  [0x1f22f, 0x1f22f],
  [0x1f237, 0x1f237],
  [0x1f30d, 0x1f30f],
  [0x1f315, 0x1f315],
  [0x1f31c, 0x1f31c],
  [0x1f321, 0x1f321],
  [0x1f324, 0x1f32c],
  [0x1f336, 0x1f336],
  [0x1f378, 0x1f378],
  [0x1f37d, 0x1f37d],
  [0x1f393, 0x1f393],
  [0x1f396, 0x1f397],
  [0x1f399, 0x1f39b],
  [0x1f39e, 0x1f39f],
  [0x1f3a7, 0x1f3a7],
  [0x1f3ac, 0x1f3ae],
  [0x1f3c2, 0x1f3c2],
  [0x1f3c4, 0x1f3c4],
  [0x1f3c6, 0x1f3c6],
  [0x1f3ca, 0x1f3ce],
  [0x1f3d4, 0x1f3e0],
  [0x1f3ed, 0x1f3ed],
  [0x1f3f3, 0x1f3f3],
  [0x1f3f5, 0x1f3f5],
  [0x1f3f7, 0x1f3f7],
  [0x1f408, 0x1f408],
  [0x1f415, 0x1f415],
  [0x1f41f, 0x1f41f],
  [0x1f426, 0x1f426],
  [0x1f43f, 0x1f43f],
  [0x1f441, 0x1f442],
  [0x1f446, 0x1f449],
  [0x1f44d, 0x1f44e],
  [0x1f453, 0x1f453],
  [0x1f46a, 0x1f46a],
  [0x1f47d, 0x1f47d],
  [0x1f4a3, 0x1f4a3],
  [0x1f4b0, 0x1f4b0],
  [0x1f4b3, 0x1f4b3],
  [0x1f4bb, 0x1f4bb],
  [0x1f4bf, 0x1f4bf],
  [0x1f4cb, 0x1f4cb],
  [0x1f4da, 0x1f4da],
  [0x1f4df, 0x1f4df],
  [0x1f4e4, 0x1f4e6],
  [0x1f4ea, 0x1f4ed],
  [0x1f4f7, 0x1f4f7],
  [0x1f4f9, 0x1f4fb],
  [0x1f4fd, 0x1f4fd],
  [0x1f508, 0x1f508],
  [0x1f50d, 0x1f50d],
  [0x1f512, 0x1f513],
  [0x1f549, 0x1f54a],
  [0x1f550, 0x1f567],
  [0x1f56f, 0x1f570],
  [0x1f573, 0x1f579],
  [0x1f587, 0x1f587],
  [0x1f58a, 0x1f58d],
  [0x1f590, 0x1f590],
  [0x1f5a5, 0x1f5a5],
  [0x1f5a8, 0x1f5a8],
  [0x1f5b1, 0x1f5b2],
  [0x1f5bc, 0x1f5bc],
  [0x1f5c2, 0x1f5c4],
  [0x1f5d1, 0x1f5d3],
  [0x1f5dc, 0x1f5de],
  [0x1f5e1, 0x1f5e1],
  [0x1f5e3, 0x1f5e3],
  [0x1f5e8, 0x1f5e8],
  [0x1f5ef, 0x1f5ef],
  [0x1f5f3, 0x1f5f3],
  [0x1f5fa, 0x1f5fa],
  [0x1f610, 0x1f610],
  [0x1f687, 0x1f687],
  [0x1f68d, 0x1f68d],
  [0x1f691, 0x1f691],
  [0x1f694, 0x1f694],
  [0x1f698, 0x1f698],
  [0x1f6ad, 0x1f6ad],
  [0x1f6b2, 0x1f6b2],
  [0x1f6b9, 0x1f6ba],
  [0x1f6bc, 0x1f6bc],
  [0x1f6cb, 0x1f6cb],
  [0x1f6cd, 0x1f6cf],
  [0x1f6e0, 0x1f6e5],
  [0x1f6e9, 0x1f6e9],
  [0x1f6f0, 0x1f6f0],
  [0x1f6f3, 0x1f6f3],
];

const EMOJI_PRESENTATION_SEQ_CODE_POINTS = flattenRanges(EMOJI_PRESENTATION_SEQ_RANGES);

// Exported for the boundary/parity test against the unicode-width 0.2.2 fixture (#10819).
export function isEmojiPresentationSeqBase(codePoint: number): boolean {
  return rangeContains(EMOJI_PRESENTATION_SEQ_CODE_POINTS, codePoint);
}

// Non-spacing and enclosing marks (Mn, Me) measure 0 under unicode-width
// 0.2, but spacing marks (Mc, e.g. U+0903) measure 1, like a base character
// — a blanket \p{M} rule diverged on lone spacing marks (#1581).
const NON_SPACING_MARK = /^(?:\p{Mn}|\p{Me})$/u;

// Spacing marks that also carry Grapheme_Extend are the exception:
// unicode-width 0.2 treats Grapheme_Extend as zero-width, so Bengali
// U+09BE and Oriya U+0B3E (both Mc) measure 0 in a cluster — কা is
// one column, not two — while U+0903 (Mc without Grapheme_Extend)
// keeps measuring 1 above (#1581, Codex review on df6e0cf15).
const GRAPHEME_EXTEND = /^\p{Grapheme_Extend}$/u;

/** Width of one code point under unicode-width 0.2: wide = 2, combining/zero-width = 0, else 1. */
function codePointWidth(character: string, codePoint: number): number {
  if (codePoint === 0x200d || codePoint === 0x200c || (codePoint >= 0xfe00 && codePoint <= 0xfe0f)) return 0;
  // Tag characters (U+E0020..U+E007F) contribute 0: a tag sequence (e.g. the
  // England flag) measures as its base alone under unicode-width 0.2 (#1431).
  if (codePoint >= 0xe0020 && codePoint <= 0xe007f) return 0;
  // Hangul jamo vowels and trailing jamo (U+1160..U+11FF) contribute 0:
  // the lead jamo carries the composed syllable's width 2, so an LV/LVT
  // cluster measures 2 in total, not per component (#1581).
  if (codePoint >= 0x1160 && codePoint <= 0x11ff) return 0;
  if (isWideCodePoint(codePoint)) return 2;
  // U+2D7F (Tifinagh consonant joiner) is Mn/Grapheme_Extend but
  // measures 1 under unicode-width 0.2.2 — the only reachable
  // mark-class point Rust admits at width 1 (#2962).
  if (codePoint === 0x2d7f) return 1;
  if (NON_SPACING_MARK.test(character) || GRAPHEME_EXTEND.test(character)) return 0;
  return 1;
}

/**
 * Width of a single grapheme cluster under unicode-width 0.2 (#761):
 * a ZWJ sequence containing a wide/emoji code point measures 2 in total
 * (not per component), and U+FE0F (emoji presentation) forces width 2
 * only when the code point immediately before it can start an emoji
 * presentation sequence (EMOJI_PRESENTATION_SEQ_RANGES) — e.g. 😀, 𠮷,
 * 👨‍👩‍👧 and ⚠️ are all width 2, matching the Rust validator, while a
 * letter followed by U+FE0F measures 1 and a lone U+FE0F measures 0
 * (#3265). (A lone U+200D measures 1 here vs 0 in Rust, but the server
 * never emits one as a cell: width-0 graphemes only ever appear as the
 * empty-symbol continuation cells, so that divergence is unreachable
 * from valid wire data.)
 * An emoji-modifier sequence (base + U+1F3FB..U+1F3FF) measures 2 as a
 * whole under unicode-width 0.2 — even when the base alone is narrow
 * (☝ U+261D is width 1, ☝🏻 is width 2) — and tag characters contribute
 * 0 via codePointWidth, so a tag sequence measures as its base (#1431,
 * #1581). U+FE0E (text presentation) immediately after a
 * default-emoji-presentation code point narrows it from 2 to 1 in the
 * plain sum below (see TEXT_PRESENTATION_RANGES); after any other code
 * point it changes nothing.
 */
function graphemeWidth(symbol: string): number {
  const characters = Array.from(symbol);
  const codePoints = characters.map((character) => character.codePointAt(0) ?? 0);
  if (codePoints.includes(0x200d)) {
    // U+FE0E narrowing applies before the ZWJ shortcut: a
    // text-presentation code point immediately followed by U+FE0E
    // counts 1, so a ZWJ sequence whose only wide point is narrowed
    // (e.g. U+2B1B U+FE0E U+200D) measures 1 under unicode-width
    // 0.2.2, not 2 (#1581, Codex review on PR #2945).
    const narrowed = new Set<number>();
    codePoints.forEach((codePoint, i) => {
      if (codePoint === 0xfe0e && i > 0 && isTextPresentationCodePoint(codePoints[i - 1])) narrowed.add(i - 1);
    });
    return codePoints.some((codePoint, i) => !narrowed.has(i) && isWideCodePoint(codePoint)) ? 2 : 1;
  }
  // U+FE0F widens only an emoji-presentable base (the code point
  // immediately before it); after any other base it measures 0 in the
  // plain sum below, so a letter+FE0F cluster measures 1 and a lone
  // U+FE0F measures 0, matching unicode-width 0.2.2 (#3265).
  const fe0fIndex = codePoints.indexOf(0xfe0f);
  if (fe0fIndex > 0 && isEmojiPresentationSeqBase(codePoints[fe0fIndex - 1])) return 2;
  const isModifier = (codePoint: number) => codePoint >= 0x1f3fb && codePoint <= 0x1f3ff;
  if (codePoints.some(isModifier)) {
    const base = characters.filter((_, i) => !isModifier(codePoints[i]));
    // A cluster of only modifiers falls through to the plain sum.
    if (base.length > 0) return 2;
  }
  return characters.reduce((total, character, i) => {
    const width = codePointWidth(character, codePoints[i]);
    // U+FE0E itself measures 0 (handled in codePointWidth); when it
    // immediately follows a text-presentation code point, that point
    // counts 1 instead of 2, so subtract the extra column (#1581).
    if (codePoints[i] === 0xfe0e && i > 0 && isTextPresentationCodePoint(codePoints[i - 1])) {
      return total + width - 1;
    }
    return total + width;
  }, 0);
}

const segmenter =
  typeof Intl !== "undefined" && Intl.Segmenter ? new Intl.Segmenter("en", { granularity: "grapheme" }) : null;

/**
 * Render a validated cell grid as text-only DOM nodes; return false on invalid wire data.
 * Width measurement matches Rust's unicode-width 0.2 semantics (see graphemeWidth)
 * and grapheme boundaries come from Intl.Segmenter.
 */
export function renderWorldSurface(root: HTMLElement, surface: ScreenSurface): boolean {
  if (
    surface?.kind !== "screen" ||
    surface.version !== 1 ||
    !Number.isInteger(surface.width) ||
    !Number.isInteger(surface.height) ||
    surface.width < 1 ||
    surface.width > MAX_SCREEN_WIDTH ||
    surface.height < 1 ||
    surface.height > MAX_SCREEN_HEIGHT ||
    !Array.isArray(surface.cells) ||
    surface.cells.length !== surface.width * surface.height ||
    surface.cells.length > MAX_SCREEN_CELLS
  ) {
    return false;
  }

  const grid = document.createElement("div");
  grid.className = "id-world-screen";
  grid.setAttribute("role", "img");
  grid.setAttribute("aria-label", `World screen, ${surface.width} columns by ${surface.height} rows`);
  grid.style.display = "grid";
  grid.style.gridTemplateColumns = `repeat(${surface.width}, 1ch)`;
  grid.style.gridTemplateRows = `repeat(${surface.height}, 1em)`;
  grid.style.width = `${surface.width}ch`;
  grid.style.whiteSpace = "pre";

  for (let index = 0; index < surface.cells.length; index++) {
    const cell = surface.cells[index];
    // No symbol-length cap: the Rust validator imposes none — only
    // exactly-one-grapheme and width 1–2 (checked below) — and a TS-only
    // cap refused Rust-valid cells such as a base letter carrying many
    // combining marks (#1581). The two validators must not disagree.
    if (
      !cell ||
      typeof cell.symbol !== "string" ||
      !isPrintableSymbol(cell.symbol) ||
      !Number.isInteger(cell.fg) ||
      !Number.isInteger(cell.bg) ||
      cell.fg < 0 ||
      cell.fg > 255 ||
      cell.bg < 0 ||
      cell.bg > 255 ||
      typeof cell.bold !== "boolean"
    ) {
      return false;
    }
    const x = index % surface.width;
    const y = Math.floor(index / surface.width);
    if (cell.columns === 0) {
      if (x === 0 || cell.symbol !== "") return false;
      const previous = surface.cells[index - 1];
      if (
        previous?.columns !== 2 ||
        previous.fg !== cell.fg ||
        previous.bg !== cell.bg ||
        previous.bold !== cell.bold
      ) {
        return false;
      }
      continue;
    }
    if (cell.columns !== 1 && cell.columns !== 2) return false;

    // ⚡ Bolt optimization: a cell whose symbol is a single Unicode scalar
    // value is always exactly one grapheme cluster — a grapheme boundary
    // sits before and after every scalar value (verified exhaustively over
    // U+0000..U+10FFFF against Intl.Segmenter). Terminal frames are
    // overwhelmingly single-scalar cells, so this skips Intl.Segmenter
    // (≈3 µs per call) and the per-cell `Array.from`/`map`/`reduce`
    // allocations on that path. The width of a one-scalar cluster is the
    // plain codePointWidth of that code point — exactly what graphemeWidth
    // reduces to when there is no U+200D, U+FE0F, modifier or U+FE0E
    // narrowing — so the general path's width check is preserved. A lone
    // surrogate (not a scalar value) keeps the general path.
    const firstCodeUnit = cell.symbol.length === 0 ? -1 : cell.symbol.charCodeAt(0);
    const codePoint = cell.symbol.length === 0 ? 0 : cell.symbol.codePointAt(0)!;
    // U+200D (ZWJ) is excluded: graphemeWidth gives it width 1 via its
    // ZWJ shortcut even though codePointWidth returns 0, so it must take
    // the general path to keep the verdict identical.
    const isSingleScalar =
      codePoint !== 0x200d &&
      ((cell.symbol.length === 1 && (firstCodeUnit < 0xd800 || firstCodeUnit > 0xdfff)) ||
        (cell.symbol.length === 2 && firstCodeUnit >= 0xd800 && firstCodeUnit <= 0xdbff));
    if (isSingleScalar) {
      if (codePoint >= 0x20 && codePoint <= 0x7e) {
        if (cell.columns !== 1) return false;
      } else if (codePointWidth(cell.symbol, codePoint) !== cell.columns) {
        return false;
      }
    } else {
      const graphemes = segmenter ? Array.from(segmenter.segment(cell.symbol)) : Array.from(cell.symbol);
      if (graphemes.length !== 1) return false;

      const terminalWidth = graphemeWidth(cell.symbol);
      if (terminalWidth !== cell.columns) return false;
    }

    if (cell.columns === 2) {
      const continuation = surface.cells[index + 1];
      if (
        x + 1 >= surface.width ||
        !continuation ||
        continuation.columns !== 0 ||
        continuation.fg !== cell.fg ||
        continuation.bg !== cell.bg ||
        continuation.bold !== cell.bold ||
        continuation.symbol !== ""
      ) {
        return false;
      }
    }

    const span = document.createElement("span");
    span.textContent = cell.symbol;
    span.style.gridColumn = `${x + 1} / span ${cell.columns}`;
    span.style.gridRow = String(y + 1);
    const foreground = xtermColor(cell.fg);
    const background = xtermColor(cell.bg);
    if (foreground) span.style.color = foreground;
    if (background) span.style.backgroundColor = background;
    if (cell.bold) span.style.fontWeight = "bold";
    grid.append(span);
  }

  root.replaceChildren(grid);
  return true;
}

/* ------------------------------------------------------------------ */
/* Screen v2: semantic tagged cells (#14731 slice 5, #6010).          */
/* Screen v1 above is unchanged: a v1 surface carries no tags, a v2   */
/* surface is a separate explicit type, and neither validator accepts */
/* the other's version — tags are never silently dropped.             */
/* ------------------------------------------------------------------ */

/** Screen format version carrying semantic tags. */
export const SCREEN_VERSION_TAGGED = 2;
/** Maximum semantic tags on one tagged cell (id-core MAX_CELL_TAGS). */
export const MAX_CELL_TAGS = 16;
/** Maximum length of one semantic tag, in bytes (id-core MAX_TAG_BYTES). */
export const MAX_TAG_BYTES = 64;

const SEMANTIC_TAG_SEGMENT = /^[a-z][a-z0-9_]*$/;

/**
 * Validate one semantic tag, mirroring id-core `validate_semantic_tag`:
 * a dotted name rooted at `sys` or `world` with at least one more
 * segment, each segment starting with a lowercase letter and using
 * only lowercase letters, digits, or underscores.
 */
export function isValidSemanticTag(tag: unknown): tag is string {
  if (typeof tag !== "string" || tag.length === 0 || tag.length > MAX_TAG_BYTES) return false;
  const segments = tag.split(".");
  if (segments.length < 2) return false;
  if (segments[0] !== "sys" && segments[0] !== "world") return false;
  return segments.every((segment) => SEMANTIC_TAG_SEGMENT.test(segment));
}

/**
 * Validate a cell's tag set: bounded, every tag valid, strictly
 * ascending with no duplicates — the one canonical wire form, so
 * diagnostics derived from the set are deterministic.
 */
export function isValidTagSet(tags: unknown): tags is string[] {
  if (!Array.isArray(tags) || tags.length > MAX_CELL_TAGS) return false;
  for (let index = 0; index < tags.length; index++) {
    if (!isValidSemanticTag(tags[index])) return false;
    if (index > 0 && !(tags[index - 1] < tags[index])) return false;
  }
  return true;
}

export type TaggedScreenCell = ScreenCell & {
  /** The cell's semantic tags, sorted and unique. Present on every v2 cell. */
  tags: string[];
};

export type ScreenSurfaceV2 = {
  kind: "screen";
  version: typeof SCREEN_VERSION_TAGGED;
  width: number;
  height: number;
  cells: TaggedScreenCell[];
};

export type ScreenPatchV2 = {
  version: typeof SCREEN_VERSION_TAGGED;
  width: number;
  height: number;
  runs: Array<{ start: number; cells: TaggedScreenCell[] }>;
};

function isValidTaggedCell(cell: unknown): cell is TaggedScreenCell {
  if (!cell || typeof cell !== "object") return false;
  const candidate = cell as TaggedScreenCell;
  return (
    typeof candidate.symbol === "string" &&
    isPrintableSymbol(candidate.symbol) &&
    Number.isInteger(candidate.fg) &&
    candidate.fg >= 0 &&
    candidate.fg <= 255 &&
    Number.isInteger(candidate.bg) &&
    candidate.bg >= 0 &&
    candidate.bg <= 255 &&
    typeof candidate.bold === "boolean" &&
    Number.isInteger(candidate.columns) &&
    candidate.columns >= 0 &&
    candidate.columns <= 2 &&
    (candidate.token === undefined ||
      candidate.token === null ||
      typeof candidate.token === "string") &&
    isValidTagSet(candidate.tags)
  );
}

/** Validate a whole Screen v2 surface: version, bounds, cells, tags. */
export function isValidScreenSurfaceV2(surface: unknown): surface is ScreenSurfaceV2 {
  if (!surface || typeof surface !== "object") return false;
  const candidate = surface as ScreenSurfaceV2;
  return (
    candidate.kind === "screen" &&
    candidate.version === SCREEN_VERSION_TAGGED &&
    Number.isInteger(candidate.width) &&
    candidate.width >= 1 &&
    candidate.width <= MAX_SCREEN_WIDTH &&
    Number.isInteger(candidate.height) &&
    candidate.height >= 1 &&
    candidate.height <= MAX_SCREEN_HEIGHT &&
    Array.isArray(candidate.cells) &&
    candidate.cells.length === candidate.width * candidate.height &&
    candidate.cells.length <= MAX_SCREEN_CELLS &&
    candidate.cells.every((cell) => isValidTaggedCell(cell))
  );
}

/**
 * Apply a Screen v2 patch, mirroring id-core `ScreenPatchV2::apply`:
 * versions and dimensions must match, runs must be ordered and in
 * bounds, and every incoming cell's tag set is validated — an invalid
 * patch returns null instead of applying with tags dropped.
 */
export function applyScreenPatchV2(previous: ScreenSurfaceV2, patch: ScreenPatchV2): ScreenSurfaceV2 | null {
  if (
    !isValidScreenSurfaceV2(previous) ||
    !patch ||
    patch.version !== SCREEN_VERSION_TAGGED ||
    patch.width !== previous.width ||
    patch.height !== previous.height ||
    !Array.isArray(patch.runs)
  ) {
    return null;
  }
  const cells = previous.cells.slice();
  let coveredUntil = 0;
  for (const run of patch.runs) {
    if (!run || !Number.isInteger(run.start) || run.start < coveredUntil || !Array.isArray(run.cells)) return null;
    if (!run.cells.every((cell) => isValidTaggedCell(cell))) return null;
    const end = run.start + run.cells.length;
    if (!Number.isSafeInteger(end) || end > cells.length) return null;
    for (let index = 0; index < run.cells.length; index++) cells[run.start + index] = run.cells[index];
    coveredUntil = end;
  }
  const next = { ...previous, cells };
  return isValidScreenSurfaceV2(next) ? next : null;
}

/** The four fallback rungs, richest first (id-core presentation_policy::Rung). */
export const FALLBACK_RUNGS = ["rich", "static", "tileset", "glyph"] as const;
export type FallbackRung = (typeof FALLBACK_RUNGS)[number];

export type FallbackCandidateData =
  | { kind: "asset"; ref: string; retainedTags: string[]; retainedFilters: string[] }
  | { kind: "glyph"; glyph: string; retainedTags: string[]; retainedFilters: string[] };

export type FallbackPlanData = {
  rich?: FallbackCandidateData | null;
  static?: FallbackCandidateData | null;
  tileset?: FallbackCandidateData | null;
  glyph: FallbackCandidateData;
};

export type ResolvedTaggedCellData = {
  index: number;
  symbol: string;
  tags: string[];
  rung: FallbackRung;
  art: { kind: "asset"; ref: string } | { kind: "glyph"; glyph: string };
  droppedTags: string[];
  droppedFilters: string[];
  /** The plan's glyph output: non-blank and never "?", at every rung. */
  fallbackGlyph: string;
};

function droppedFilterOccurrences(source: string[], retained: string[]): string[] {
  const counts = new Map<string, number>();
  for (const filter of retained) counts.set(filter, (counts.get(filter) ?? 0) + 1);
  const dropped: string[] = [];
  for (const filter of source) {
    const count = counts.get(filter) ?? 0;
    if (count === 0) dropped.push(filter);
    else counts.set(filter, count - 1);
  }
  return dropped;
}

function candidateIsValidFor(
  rung: FallbackRung,
  candidate: FallbackCandidateData,
  sourceTags: ReadonlySet<string>,
  sourceFilters: string[],
): boolean {
  if (rung === "glyph" ? candidate.kind !== "glyph" : candidate.kind !== "asset") return false;
  if (candidate.kind === "glyph" && (candidate.glyph.trim() === "" || candidate.glyph === "?")) return false;
  if (!candidate.retainedTags.every((tag) => sourceTags.has(tag))) return false;
  // Retained filters must be occurrences of the source chain.
  const available = new Map<string, number>();
  for (const filter of sourceFilters) available.set(filter, (available.get(filter) ?? 0) + 1);
  for (const filter of candidate.retainedFilters) {
    const count = available.get(filter) ?? 0;
    if (count === 0) return false;
    available.set(filter, count - 1);
  }
  return true;
}

/**
 * Connect the four-rung fallback resolver to one tagged cell,
 * mirroring id-core `ScreenSurfaceV2::resolve_cell` +
 * `FallbackPlan::resolve`: select the richest available rung at or
 * below `startAt`, and report the tags/filters that rung drops, in
 * deterministic order (tags sorted, filters in source order). Returns
 * null when the plan does not validate against the cell — a renderer
 * must never present an unvalidated resolution.
 */
export function resolveTaggedCell(
  cell: TaggedScreenCell,
  index: number,
  plan: FallbackPlanData,
  startAt: FallbackRung,
  sourceFilters: string[],
): ResolvedTaggedCellData | null {
  if (!isValidTaggedCell(cell) || !plan || plan.glyph?.kind !== "glyph") return null;
  if (plan.glyph.glyph.trim() === "" || plan.glyph.glyph === "?") return null;
  const sourceTags = new Set(cell.tags);
  const candidates: Array<[FallbackRung, FallbackCandidateData | null | undefined]> = [
    ["rich", plan.rich],
    ["static", plan.static],
    ["tileset", plan.tileset],
    ["glyph", plan.glyph],
  ];
  const startIndex = FALLBACK_RUNGS.indexOf(startAt);
  for (const [rung, candidate] of candidates) {
    if (FALLBACK_RUNGS.indexOf(rung) < startIndex || !candidate) continue;
    if (!candidateIsValidFor(rung, candidate, sourceTags, sourceFilters)) return null;
    const retainedTags = new Set(candidate.retainedTags);
    return {
      index,
      symbol: cell.symbol,
      tags: [...cell.tags],
      rung,
      art:
        candidate.kind === "asset"
          ? { kind: "asset", ref: candidate.ref }
          : { kind: "glyph", glyph: candidate.glyph },
      droppedTags: cell.tags.filter((tag) => !retainedTags.has(tag)),
      droppedFilters: droppedFilterOccurrences(sourceFilters, candidate.retainedFilters),
      fallbackGlyph: plan.glyph.glyph,
    };
  }
  return null;
}

/**
 * Tag-preserving render data for a whole Screen v2 surface: one
 * resolved record per cell that has a plan, in row-major order. This
 * is the web rendering path's input — a DOM presenter renders `symbol`
 * (or the resolved art) with `droppedTags`/`droppedFilters` as the
 * presentation diagnostic, exactly as the Rust path does.
 */
export function taggedRenderData(
  surface: ScreenSurfaceV2,
  plans: ReadonlyMap<number, FallbackPlanData>,
  startAt: FallbackRung,
  sourceFilters: string[],
): ResolvedTaggedCellData[] | null {
  if (!isValidScreenSurfaceV2(surface)) return null;
  const out: ResolvedTaggedCellData[] = [];
  for (let index = 0; index < surface.cells.length; index++) {
    const plan = plans.get(index);
    if (!plan) continue;
    const resolved = resolveTaggedCell(surface.cells[index], index, plan, startAt, sourceFilters);
    if (!resolved) return null;
    out.push(resolved);
  }
  return out;
}
