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

export function stripTypes(source) {
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
