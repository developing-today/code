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

function removeInterfaces(src) {
  const re = /(?:^|\n)[ \t]*(?:export\s+)?interface\s+[A-Za-z_$][\w$]*[^{]*\{/g;
  let out = "";
  let last = 0;
  let m;
  while ((m = re.exec(src)) !== null) {
    const brace = m.index + m[0].length - 1;
    const end = removeBalanced(src, brace);
    out += src.slice(last, m.index);
    last = end;
    re.lastIndex = end;
  }
  return out + src.slice(last);
}

function removeTypeAliases(src) {
  const re = /(?:^|\n)[ \t]*(?:export\s+)?type\s+[A-Za-z_$][\w$]*(?:\s*<[^>]*>)?\s*=\s*/g;
  let out = "";
  let last = 0;
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
    out += src.slice(last, m.index);
    last = i + 1;
    re.lastIndex = last;
  }
  return out + src.slice(last);
}

// --- Syntax-aware segmentation -------------------------------------------
// The regex passes below must never rewrite text inside string literals,
// template-literal text, or comments: a reader's edit such as
// console.log("copy as script") is data, not an `as` cast. segmentize()
// splits the source into code and protected segments; the passes run on
// code segments only and the result is reassembled in order. Template
// ${...} interpolations are code and are segmented recursively.

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

function segmentize(src) {
  const segs = [];
  let code = "";
  const flushCode = () => {
    if (code) {
      segs.push({ code: true, text: code });
      code = "";
    }
  };
  const prot = (text) => {
    flushCode();
    if (text) segs.push({ code: false, text });
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    const n = src[i + 1];
    if (c === "/" && n === "/") {
      let j = src.indexOf("\n", i);
      if (j < 0) j = src.length;
      prot(src.slice(i, j));
      i = j;
    } else if (c === "/" && n === "*") {
      let j = src.indexOf("*/", i + 2);
      j = j < 0 ? src.length : j + 2;
      prot(src.slice(i, j));
      i = j;
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
      i++;
    }
  }
  flushCode();
  return segs;
}

function stripCode(source) {
  if (/^\s*(?:export\s+)?(?:enum|namespace)\s/m.test(source) || /constructor\s*\([^)]*\b(?:private|public|protected|readonly)\b/.test(source)) {
    throw new Error(
      "strip-types: enums, namespaces and parameter properties are not " +
        "erasable and are not supported in the browser demo — remove them and run again",
    );
  }
  let src = removeInterfaces(source);
  src = removeTypeAliases(src);
  // `as T` casts.
  src = src.replace(new RegExp(String.raw`\s+as\s+` + TYPE, "g"), "");
  // Variable annotations: const/let/var name: T (=|;)
  src = src.replace(
    new RegExp(String.raw`(\b(?:const|let|var)\s+[A-Za-z_$][\w$]*)\s*:\s*` + TYPE + String.raw`(?=\s*[=;])`, "g"),
    "$1",
  );
  // Parameter annotations and optional markers, inside parens: (a: T, b?: U)
  src = src.replace(
    new RegExp(String.raw`([(,]\s*[A-Za-z_$][\w$]*)\?\s*:\s*` + TYPE + String.raw`(?=\s*[,)=])`, "g"),
    "$1",
  );
  src = src.replace(
    new RegExp(String.raw`([(,]\s*[A-Za-z_$][\w$]*)\s*:\s*` + TYPE + String.raw`(?=\s*[,)=])`, "g"),
    "$1",
  );
  // Return annotations: ): T {  and  ): T =>
  src = src.replace(
    new RegExp(String.raw`\)\s*:\s*` + TYPE + String.raw`(?=\s*(?:\{|=>))`, "g"),
    ")",
  );
  return src;
}

export function stripTypes(source) {
  return segmentize(source)
    .map((s) => (s.code ? stripCode(s.text) : s.text))
    .join("");
}
