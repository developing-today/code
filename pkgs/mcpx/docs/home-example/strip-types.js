// A strip-only TypeScript transform for the home-page example (#1876).
//
// Why strip-only: the generated mcpx client is written to run under Node's
// strip-only TypeScript mode (docs/story.md, "Runtime portability"), and
// the docs examples are written the same way. The page needs the same
// favour for the reader's *edits* to example.ts — change a line, press Run
// — without shipping a compiler. (Candidates weighed in the issue: sucrase
// is the small real stripper, esbuild-wasm and tsc are megabytes. When the
// site build lands in #1941 it may swap this module for sucrase behind the
// same signature; the component does not care.)
//
// What this handles — the erasable subset docs examples use:
//   - `interface X { ... }` and `type X = ...;` declarations (removed)
//   - annotations on variable declarations: `const x: T = ...`
//   - parameter and return annotations: `function f(a: T): R { ... }`,
//     `(a: T) => ...`
//   - `as T` casts (removed)
//   - optional parameters `a?: T`
//
// What it refuses, loudly, rather than mistranslating: enums, namespaces,
// parameter properties (`constructor(private x)`), and decorators. An
// unsupported edit produces a clear error in the output pane — never a
// silently different program. Anything outside the subset above may also
// fail to strip; the failure mode is a syntax error on import, shown in
// the pane, not wrong behaviour.

const TYPE = String.raw`[A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*(?:\s*<[^<>;{}()]*>)?(?:\s*\[\])*(?:\s*\|\s*[A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*(?:\s*<[^<>;{}()]*>)?(?:\s*\[\])*)*\??`;

function removeBalanced(src, start) {
  // start is at the opening '{' — return the index just past its match.
  let depth = 0;
  for (let i = start; i < src.length; i++) {
    if (src[i] === "{") depth++;
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) return i + 1;
    }
  }
  throw new Error("strip-types: unbalanced braces in type declaration");
}

function interfaceSpans(src) {
  const re = /(?:^|\n)[ \t]*(?:export\s+)?interface\s+[A-Za-z_$][\w$]*[^{]*\{/g;
  const spans = [];
  let m;
  while ((m = re.exec(src)) !== null) {
    const brace = m.index + m[0].length - 1;
    const end = removeBalanced(src, brace);
    spans.push([m.index, end]);
    re.lastIndex = end;
  }
  return spans;
}

function typeAliasSpans(src) {
  const re = /(?:^|\n)[ \t]*(?:export\s+)?type\s+[A-Za-z_$][\w$]*(?:\s*<[^>]*>)?\s*=\s*/g;
  const spans = [];
  let m;
  while ((m = re.exec(src)) !== null) {
    // Scan to the terminating ';' at depth 0 (object types contain braces).
    let depth = 0;
    let i = re.lastIndex;
    for (; i < src.length; i++) {
      const c = src[i];
      if (c === "{" || c === "(" || c === "[") depth++;
      else if (c === "}" || c === ")" || c === "]") depth--;
      else if (c === ";" && depth === 0) break;
    }
    spans.push([m.index, i + 1]);
    re.lastIndex = i + 1;
  }
  return spans;
}

/**
 * Deletion spans for a regex pass whose replacement keeps a prefix of
 * each match: kept(m) is how many leading characters survive (the
 * parenthesised group for annotation passes, 1 for the return pass's
 * ')', 0 for `as` casts, whose whole match is deleted).
 */
function passSpans(src, re, kept) {
  const spans = [];
  let m;
  while ((m = re.exec(src)) !== null) {
    const k = typeof kept === "function" ? kept(m) : kept;
    if (m[0].length > k) spans.push([m.index + k, m.index + m[0].length]);
    if (m[0].length === 0) re.lastIndex++;
  }
  return spans;
}

// --- Syntax-aware segmentation -------------------------------------------
// The regex passes below must never rewrite text inside string literals,
// template-literal text, or comments: a reader's edit such as
// console.log("copy as script") is data, not an `as` cast. segmentize()
// splits the source into code and protected segments; the structural
// passes then run on the whole source with protected segments masked
// to spaces (lengths preserved), so a construct split by a literal —
// a parameter list with a string default — is still seen whole.
// Template ${...} interpolations are code and are segmented
// recursively. Regex literals are protected too (see regexAllowedHere).

function skipQuoted(src, i, quote) {
  // i is at the opening quote; return the index just past the closer.
  i++;
  while (i < src.length) {
    if (src[i] === "\\") i += 2;
    else if (src[i] === quote) return i + 1;
    else i++;
  }
  return src.length;
}

function scanExprEnd(src, i) {
  // i is just past a template '${'; return the index of its matching '}',
  // skipping nested strings, templates, and comments.
  let depth = 1;
  while (i < src.length) {
    const c = src[i];
    if (c === '"' || c === "'") i = skipQuoted(src, i, c);
    else if (c === "`") i = skipTemplate(src, i);
    else if (c === "/" && src[i + 1] === "/") {
      const j = src.indexOf("\n", i);
      i = j < 0 ? src.length : j;
    } else if (c === "/" && src[i + 1] === "*") {
      const j = src.indexOf("*/", i + 2);
      i = j < 0 ? src.length : j + 2;
    } else if (c === "{") {
      depth++;
      i++;
    } else if (c === "}") {
      depth--;
      if (depth === 0) return i;
      i++;
    } else i++;
  }
  return src.length;
}

function skipTemplate(src, i) {
  // i is at the opening backtick; return the index just past the closer.
  i++;
  while (i < src.length) {
    if (src[i] === "\\") i += 2;
    else if (src[i] === "`") return i + 1;
    else if (src[i] === "$" && src[i + 1] === "{") i = scanExprEnd(src, i + 2) + 1;
    else i++;
  }
  return src.length;
}

// A '/' opens a regex literal only where a value cannot stand: at the
// start, after an operator or opener, or after a keyword like return.
// After an operand (identifier, number, ')', ']', a string) it is
// division. segmentize() tracks just enough history for that call —
// regex literals are protected segments, like strings, because a
// pattern can contain text the stripping passes would otherwise eat
// (/ as string / lost its pattern to the `as` pass).
const REGEX_KEYWORDS = new Set([
  "return", "typeof", "instanceof", "in", "of", "new", "delete",
  "void", "case", "do", "else", "yield", "await", "throw",
]);

function regexAllowedHere(history) {
  const tail = history.replace(/\s+$/, "");
  if (tail === "") return true;
  const last = tail[tail.length - 1];
  if (/[\w$]/.test(last)) {
    const word = /([A-Za-z_$][\w$]*)$/.exec(tail)?.[1] ?? "";
    return REGEX_KEYWORDS.has(word);
  }
  // Operand enders: a value just ended, so '/' divides.
  if (last === ")" || last === "]" || last === "}" || last === '"') return false;
  return true;
}

function scanRegexEnd(src, i) {
  // i is at the opening '/'; return the index just past the flags, or
  // -1 if this is not a well-formed literal on one line.
  let inClass = false;
  let j = i + 1;
  while (j < src.length) {
    const c = src[j];
    if (c === "\\") j += 2;
    else if (c === "\n") return -1;
    else if (c === "[") { inClass = true; j++; }
    else if (c === "]") { inClass = false; j++; }
    else if (c === "/" && !inClass) {
      j++;
      while (j < src.length && /[a-z]/.test(src[j])) j++;
      return j;
    } else j++;
  }
  return -1;
}

function segmentize(src) {
  const segs = [];
  let code = "";
  // The recent code history, for the regex/division decision: code
  // characters as scanned, '"' as the marker for a just-ended operand
  // segment (string, template, regex), ' ' for a comment.
  let history = "";
  const flushCode = () => {
    if (code) {
      segs.push({ code: true, text: code });
      code = "";
    }
  };
  const prot = (text, marker = '"') => {
    flushCode();
    if (text) segs.push({ code: false, text });
    history += marker;
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    const n = src[i + 1];
    if (c === "/" && n === "/") {
      let j = src.indexOf("\n", i);
      if (j < 0) j = src.length;
      prot(src.slice(i, j), " ");
      i = j;
    } else if (c === "/" && n === "*") {
      let j = src.indexOf("*/", i + 2);
      j = j < 0 ? src.length : j + 2;
      prot(src.slice(i, j), " ");
      i = j;
    } else if (c === "/" && regexAllowedHere(history)) {
      const j = scanRegexEnd(src, i);
      if (j > 0) {
        prot(src.slice(i, j));
        i = j;
      } else {
        code += c;
        history += c;
        i++;
      }
    } else if (c === '"' || c === "'") {
      const j = skipQuoted(src, i, c);
      prot(src.slice(i, j));
      i = j;
    } else if (c === "`") {
      // Literal chunks are protected; ${...} expressions are code.
      let j = i + 1;
      let chunkStart = i;
      while (j < src.length) {
        if (src[j] === "\\") {
          j += 2;
          continue;
        }
        if (src[j] === "`") {
          j++;
          break;
        }
        if (src[j] === "$" && src[j + 1] === "{") {
          prot(src.slice(chunkStart, j + 2)); // chunk incl. the '${'
          const end = scanExprEnd(src, j + 2);
          for (const s of segmentize(src.slice(j + 2, end))) segs.push(s);
          chunkStart = end; // the closing '}' starts the next chunk
          j = end + 1;
          continue;
        }
        j++;
      }
      prot(src.slice(chunkStart, j));
      i = j;
    } else {
      code += c;
      history += c;
      i++;
    }
  }
  flushCode();
  return segs;
}

// --- Parameter annotations ------------------------------------------------
// The naive pass — "an identifier after '(' or ',' followed by ': Type'
// and then ',', ')' or '='" — cannot tell a parameter list from an object
// literal inside a call: `{ first: 0, actual: expected }` matches it, and
// stripping there silently rewrites a property into shorthand, changing
// what the program computes. So annotations are stripped only inside
// paren groups that are actually parameter lists, and only at the
// group's own depth (an object literal in a default value is nested,
// and its properties are data, not annotations).

const CONTROL_KEYWORDS = new Set(["if", "for", "while", "switch", "catch", "with"]);

function findMatchingParen(src, open) {
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === "(") depth++;
    else if (src[i] === ")") {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

function isParamList(src, open, close) {
  // What follows the group decides: an optional return annotation, then
  // a body ('{') or an arrow ('=>'). A call is followed by neither.
  let rest = src.slice(close + 1).replace(/^\s+/, "");
  if (rest.startsWith(":")) {
    const t = new RegExp(String.raw`^:\s*` + TYPE).exec(rest);
    if (!t) return false;
    rest = rest.slice(t[0].length).replace(/^\s+/, "");
  }
  if (!rest.startsWith("{") && !rest.startsWith("=>")) return false;
  // Control-flow parens are followed by a body too, but hold no params.
  const before = /(?:^|[^\w$.])([A-Za-z_$][\w$]*)\s*(?:<[^<>]*>)?\s*$/.exec(
    src.slice(0, open),
  );
  return !(before && CONTROL_KEYWORDS.has(before[1]));
}

function maskNested(text) {
  // Length-preserving: characters inside nested (), {} or [] become
  // spaces (delimiters stay), so a match found in the masked text maps
  // to the same span in the original.
  let out = "";
  let depth = 0;
  for (const c of text) {
    if (c === "(" || c === "{" || c === "[") {
      out += c;
      depth++;
    } else if (c === ")" || c === "}" || c === "]") {
      depth--;
      out += c;
    } else out += depth > 0 ? " " : c;
  }
  return out;
}

const PARAM_PASSES = [
  new RegExp(String.raw`([(,]\s*[A-Za-z_$][\w$]*)\?\s*:\s*` + TYPE + String.raw`(?=\s*[,)=])`, "g"),
  new RegExp(String.raw`([(,]\s*[A-Za-z_$][\w$]*)\s*:\s*` + TYPE + String.raw`(?=\s*[,)=])`, "g"),
];

function groupParamSpans(inner, base, spans) {
  // The passes anchor on the '(' or ',' before a parameter and look
  // ahead to the ',', ')' or '=' after its type; the group's own parens
  // are not part of `inner`, so matches are found in a copy lent a
  // synthetic comma before and a synthetic ')' after. maskNested keeps
  // nested spans (object literals in defaults) from matching, at the
  // same offsets. Span coordinates are converted back by dropping the
  // one synthetic leading character.
  const masked = maskNested("," + inner + ")");
  for (const re of PARAM_PASSES) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(masked)) !== null) {
      spans.push([base + m.index + m[1].length - 1, base + m.index + m[0].length - 1]);
    }
  }
}

function collectParamSpans(src, base, spans) {
  let i = 0;
  while (i < src.length) {
    const open = src.indexOf("(", i);
    if (open < 0) return;
    const close = findMatchingParen(src, open);
    if (close < 0) return;
    // Nested groups first: an arrow in a default value or an argument
    // has its own parameter list, judged on its own merits.
    collectParamSpans(src.slice(open + 1, close), base + open + 1, spans);
    if (isParamList(src, open, close)) {
      groupParamSpans(src.slice(open + 1, close), base + open + 1, spans);
    }
    i = close + 1;
  }
}

// The structural passes run on the WHOLE source with protected
// segments masked to spaces (newlines kept, lengths preserved), never
// on isolated segments: a parameter list split by a string default —
// function greet(name: string = "world") — is one list, and only the
// whole-source view can see it. Masking is length-preserving, so a
// deletion span found in the masked text is a span of the original;
// every pass below only ever deletes (annotations, casts, type
// declarations), so the transform is: collect spans, merge, cut.
function maskProtected(source) {
  return segmentize(source)
    .map((s) => (s.code ? s.text : s.text.replace(/[^\n]/g, " ")))
    .join("");
}

/**
 * The [start, end) ranges of the original that are protected literals
 * or comments. A deletion span must never cut into one: a pass can
 * legitimately reach into a masked literal — the `as` pass's leading
 * \\s+ happily eats the spaces a masked string became — and cutting
 * there would delete the literal itself from the output.
 */
function protectedRanges(source) {
  const ranges = [];
  let offset = 0;
  for (const s of segmentize(source)) {
    if (!s.code) ranges.push([offset, offset + s.text.length]);
    offset += s.text.length;
  }
  return ranges;
}

function subtractRanges(spans, ranges, source) {
  const out = [];
  for (const [a, b] of spans) {
    let pieces = [[a, b]];
    for (const [pa, pb] of ranges) {
      const next = [];
      for (const [x, y] of pieces) {
        if (pb <= x || pa >= y) {
          next.push([x, y]);
          continue;
        }
        if (pa > x) next.push([x, pa]);
        if (pb < y) next.push([pb, y]);
      }
      pieces = next;
    }
    // A piece that is only whitespace is dropped too: splitting a span
    // at a literal can leave the space before the literal as its own
    // piece (`= "x" as string`), and cutting it would weld the literal
    // to whatever precedes it. No real deletion is whitespace-only —
    // every pass deletes at least a colon, a keyword, or a brace.
    out.push(
      ...pieces.filter(
        ([x, y]) => y > x && !/^\s*$/.test(source.slice(x, y)),
      ),
    );
  }
  return out;
}

export function stripTypes(source) {
  const masked = maskProtected(source);
  if (
    /^\s*(?:export\s+)?(?:enum|namespace)\s/m.test(masked) ||
    /constructor\s*\([^)]*\b(?:private|public|protected|readonly)\b/.test(masked)
  ) {
    throw new Error(
      "strip-types: enums, namespaces and parameter properties are not " +
        "erasable and are not supported in the browser demo — remove them and run again",
    );
  }
  // Type and interface declarations are removed whole — including any
  // literals inside them (`type Mode = "fast" | "slow";` must not leave
  // `"fast""slow"` behind). Operand protection below applies only to
  // the annotation/cast passes, never to a declaration span.
  const declSpans = [
    ...interfaceSpans(masked),
    ...typeAliasSpans(masked),
  ];
  const spans = [
    // `as T` casts.
    ...passSpans(masked, new RegExp(String.raw`\s+as\s+` + TYPE, "g"), 0),
    // Variable annotations: const/let/var name: T (=|;)
    ...passSpans(
      masked,
      new RegExp(
        String.raw`(\b(?:const|let|var)\s+[A-Za-z_$][\w$]*)\s*:\s*` +
          TYPE +
          String.raw`(?=\s*[=;])`,
        "g",
      ),
      (m) => m[1].length,
    ),
    // Return annotations: ): T {  and  ): T =>
    ...passSpans(
      masked,
      new RegExp(String.raw`\)\s*:\s*` + TYPE + String.raw`(?=\s*(?:\{|=>))`, "g"),
      1,
    ),
  ];
  collectParamSpans(masked, 0, spans);

  // No annotation/cast deletion may cut into a protected literal or
  // comment; declaration spans are exempt (see declSpans above).
  const cut = [
    ...declSpans,
    ...subtractRanges(spans, protectedRanges(source), source),
  ];
  cut.sort((a, b) => a[0] - b[0]);
  spans.length = 0;
  spans.push(...cut);
  let out = "";
  let last = 0;
  for (const [a, b] of spans) {
    if (a >= last) {
      out += source.slice(last, a);
      last = b;
    } else if (b > last) {
      last = b;
    }
  }
  return out + source.slice(last);
}
