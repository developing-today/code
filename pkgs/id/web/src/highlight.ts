/**
 * Syntax highlighting and line numbers for ProseMirror code_block nodes.
 *
 * Uses prosemirror-highlight (decoration management + caching) with Shiki
 * (TextMate grammars, 200+ languages) as the highlighting engine.
 *
 * Line numbers are added via prosemirror-highlight's withLineNumbers() wrapper,
 * which inserts widget decorations at the start of each logical line.
 *
 * Language detection: filename extension → Shiki language ID, with manual
 * override support via code_block node's `language` attribute. In rich /
 * markdown mode the codec stores a fence's info string in the node's
 * `params` attribute instead (#1479); the extractor resolves that to a
 * Shiki language per block, falling back to the filename language.
 */

import { createHighlightPlugin, type Parser, withLineNumbers } from "prosemirror-highlight";
import { createParser } from "prosemirror-highlight/shiki";
import type { Plugin } from "prosemirror-state";

// ---------------------------------------------------------------------------
// Language Detection
// ---------------------------------------------------------------------------

/**
 * Maps file extensions to Shiki language identifiers.
 * Covers all extensions from content_mode.rs that map to Raw mode.
 */
export const EXT_TO_LANG: Record<string, string> = {
  // JavaScript / TypeScript
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  ts: "typescript",
  mts: "typescript",
  cts: "typescript",
  jsx: "jsx",
  tsx: "tsx",
  // Systems
  rs: "rust",
  go: "go",
  c: "c",
  cpp: "cpp",
  h: "c",
  hpp: "cpp",
  cs: "csharp",
  swift: "swift",
  kt: "kotlin",
  scala: "scala",
  java: "java",
  // Scripting
  py: "python",
  rb: "ruby",
  php: "php",
  pl: "perl",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  fish: "fish",
  ps1: "powershell",
  bat: "bat",
  cmd: "bat",
  // Web
  html: "html",
  htm: "html",
  css: "css",
  scss: "scss",
  sass: "sass",
  less: "less",
  // Data / Config
  json: "json",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  xml: "xml",
  ini: "ini",
  cfg: "ini",
  conf: "ini",
  // Build
  cmake: "cmake",
  gradle: "groovy",
  // Query / Schema
  sql: "sql",
  graphql: "graphql",
  proto: "protobuf",
  // Docs
  md: "markdown",
  markdown: "markdown",
  // Nix
  nix: "nix",
  // Misc
  lua: "lua",
  r: "r",
  dart: "dart",
  zig: "zig",
  ex: "elixir",
  exs: "elixir",
  erl: "erlang",
  hs: "haskell",
  ml: "ocaml",
  vim: "viml",
  tf: "hcl",
  // Lock / sum / mod — no highlighting
};

/**
 * Special filenames that map to specific languages regardless of extension.
 */
export const FILENAME_TO_LANG: Record<string, string> = {
  dockerfile: "dockerfile",
  makefile: "makefile",
  justfile: "just",
  cmakelists: "cmake",
  gemfile: "ruby",
  rakefile: "ruby",
  vagrantfile: "ruby",
};

/**
 * Detect the Shiki language identifier from a filename.
 *
 * Priority:
 * 1. Dotfile match (.gitignore, .env, .editorconfig, ...)
 * 2. Extension match from EXT_TO_LANG map — a known extension wins over a
 *    special-name stem ("justfile.rs" is Rust, not a Justfile)
 * 3. Special filename match (case-insensitive), only when the extension is
 *    absent or unknown ("Dockerfile", "Dockerfile.dev", "CMakeLists.txt")
 * 4. undefined (no highlighting)
 *
 * @param filename - The filename (may include path components)
 * @returns Shiki language identifier, or undefined if unknown
 */
export function detectLanguage(filename: string): string | undefined {
  // Extract basename (handle paths)
  const basename = filename.split("/").pop() ?? filename;
  const basenameLower = basename.toLowerCase();

  // Dotfiles: .gitignore, .env, .editorconfig, .prettierrc, .eslintrc
  if (basenameLower.startsWith(".")) {
    const dotName = basenameLower.slice(1);
    if (dotName === "gitignore" || dotName === "dockerignore") return "gitignore";
    if (dotName === "env" || dotName.startsWith("env.")) return "dotenv";
    if (dotName === "editorconfig") return "properties";
    // .prettierrc, .eslintrc — usually JSON
    if (dotName === "prettierrc" || dotName === "eslintrc") return "json";
  }

  // A known extension wins over a special-name stem:
  // "makefile.md" is Markdown documentation, not a Makefile.
  const ext = basename.includes(".") ? basename.split(".").pop()?.toLowerCase() : undefined;
  // Own-key check (#2787): a plain lookup also matches inherited
  // Object.prototype members, so "notes.constructor" / "notes.__proto__"
  // returned the Object constructor / prototype object as a "language".
  if (ext && Object.hasOwn(EXT_TO_LANG, ext)) {
    return EXT_TO_LANG[ext];
  }

  // Check special filenames (case-insensitive, without extension).
  // Only reached when the extension is absent or unknown, so
  // "Dockerfile.dev" is still a Dockerfile and "CMakeLists.txt" is CMake.
  const nameWithoutExt = basenameLower.includes(".")
    ? basenameLower.slice(0, basenameLower.lastIndexOf("."))
    : basenameLower;

  if (Object.hasOwn(FILENAME_TO_LANG, nameWithoutExt)) {
    return FILENAME_TO_LANG[nameWithoutExt];
  }
  // Full basename (e.g., "Dockerfile" with no ext)
  if (Object.hasOwn(FILENAME_TO_LANG, basenameLower)) {
    return FILENAME_TO_LANG[basenameLower];
  }

  return undefined;
}

// ---------------------------------------------------------------------------
// Shiki Highlighter (lazy singleton)
// ---------------------------------------------------------------------------

/** Shiki HighlighterCore type — imported lazily to avoid bundling the full types. */
type ShikiHighlighter = any;

/**
 * Static language grammar registry.
 *
 * Dynamic imports like `import('@shikijs/langs/${lang}')` don't work in this
 * embedded app (assets are bundled into the Rust binary). Instead, we import
 * the most common language grammars statically via explicit import() calls
 * that Bun can resolve and bundle at build time.
 *
 * Every language detectLanguage() can return must have an entry here —
 * otherwise ensureLanguage() marks it loaded without a grammar and it
 * renders as plain text forever (guarded by a test in highlight.test.ts).
 * Sole exception: "gitignore" — @shikijs/langs in shiki 4.5.0 ships no
 * gitignore grammar, so .gitignore/.dockerignore stay unhighlighted
 * until one exists upstream (tracked in the issue that added this note).
 */
export const LANG_IMPORTS: Record<string, () => Promise<any>> = {
  javascript: () => import("@shikijs/langs/javascript"),
  typescript: () => import("@shikijs/langs/typescript"),
  jsx: () => import("@shikijs/langs/jsx"),
  tsx: () => import("@shikijs/langs/tsx"),
  rust: () => import("@shikijs/langs/rust"),
  python: () => import("@shikijs/langs/python"),
  go: () => import("@shikijs/langs/go"),
  c: () => import("@shikijs/langs/c"),
  cpp: () => import("@shikijs/langs/cpp"),
  csharp: () => import("@shikijs/langs/csharp"),
  java: () => import("@shikijs/langs/java"),
  kotlin: () => import("@shikijs/langs/kotlin"),
  swift: () => import("@shikijs/langs/swift"),
  ruby: () => import("@shikijs/langs/ruby"),
  php: () => import("@shikijs/langs/php"),
  bash: () => import("@shikijs/langs/bash"),
  fish: () => import("@shikijs/langs/fish"),
  powershell: () => import("@shikijs/langs/powershell"),
  bat: () => import("@shikijs/langs/bat"),
  html: () => import("@shikijs/langs/html"),
  css: () => import("@shikijs/langs/css"),
  scss: () => import("@shikijs/langs/scss"),
  sass: () => import("@shikijs/langs/sass"),
  less: () => import("@shikijs/langs/less"),
  json: () => import("@shikijs/langs/json"),
  yaml: () => import("@shikijs/langs/yaml"),
  toml: () => import("@shikijs/langs/toml"),
  xml: () => import("@shikijs/langs/xml"),
  sql: () => import("@shikijs/langs/sql"),
  graphql: () => import("@shikijs/langs/graphql"),
  protobuf: () => import("@shikijs/langs/protobuf"),
  markdown: () => import("@shikijs/langs/markdown"),
  dockerfile: () => import("@shikijs/langs/dockerfile"),
  makefile: () => import("@shikijs/langs/makefile"),
  nix: () => import("@shikijs/langs/nix"),
  lua: () => import("@shikijs/langs/lua"),
  perl: () => import("@shikijs/langs/perl"),
  scala: () => import("@shikijs/langs/scala"),
  dart: () => import("@shikijs/langs/dart"),
  elixir: () => import("@shikijs/langs/elixir"),
  haskell: () => import("@shikijs/langs/haskell"),
  ini: () => import("@shikijs/langs/ini"),
  groovy: () => import("@shikijs/langs/groovy"),
  cmake: () => import("@shikijs/langs/cmake"),
  r: () => import("@shikijs/langs/r"),
  zig: () => import("@shikijs/langs/zig"),
  erlang: () => import("@shikijs/langs/erlang"),
  ocaml: () => import("@shikijs/langs/ocaml"),
  viml: () => import("@shikijs/langs/viml"),
  hcl: () => import("@shikijs/langs/hcl"),
  just: () => import("@shikijs/langs/just"),
  dotenv: () => import("@shikijs/langs/dotenv"),
  properties: () => import("@shikijs/langs/properties"),
};

let highlighterPromise: Promise<ShikiHighlighter> | null = null;
const loadedLangs = new Set<string>();

/**
 * Get or create the shared Shiki highlighter instance.
 * Uses the JavaScript regex engine (no WASM) for broad compatibility.
 * Starts with no languages loaded — they are added on demand via static imports.
 */
async function getHighlighter(): Promise<ShikiHighlighter> {
  if (!highlighterPromise) {
    highlighterPromise = (async () => {
      const { createHighlighterCore } = await import("shiki/core");
      const { createJavaScriptRegexEngine } = await import("shiki/engine/javascript");

      return createHighlighterCore({
        themes: [import("@shikijs/themes/vitesse-black")],
        langs: [],
        engine: createJavaScriptRegexEngine(),
      });
    })();
  }
  return highlighterPromise;
}

/**
 * Ensure a language grammar is loaded in the highlighter.
 * Uses the static LANG_IMPORTS registry instead of dynamic template imports.
 * No-op if the language is already loaded or not in the registry.
 */
async function ensureLanguage(highlighter: ShikiHighlighter, lang: string): Promise<void> {
  if (loadedLangs.has(lang)) return;

  // Own-key check (#2787): `lang` can be an authored code-block language
  // attribute; a plain lookup returns Object.prototype members for
  // "constructor"/"__proto__", which were then called as import functions.
  const importFn = Object.hasOwn(LANG_IMPORTS, lang) ? LANG_IMPORTS[lang] : undefined;
  if (!importFn) {
    // Language not in our static registry — mark as loaded to avoid retries
    loadedLangs.add(lang);
    return;
  }

  try {
    const langModule = await importFn();
    const langDefs = Array.isArray(langModule.default) ? langModule.default : [langModule.default];
    await highlighter.loadLanguage(...langDefs);
    loadedLangs.add(lang);
  } catch {
    // Grammar failed to load — mark as loaded to avoid repeated attempts
    loadedLangs.add(lang);
  }
}

/**
 * Author-spelling aliases for fenced-code info strings that are neither
 * Shiki language IDs (LANG_IMPORTS keys) nor file extensions in
 * EXT_TO_LANG.
 */
export const INFO_STRING_ALIASES: Record<string, string> = {
  "c++": "cpp",
  "c#": "csharp",
  shell: "bash",
};

/**
 * Resolve a fenced code block's info string to a Shiki language ID.
 *
 * The language is the info string's first word — trailing attribute
 * words ("rust ignore") must not break resolution. Author spellings
 * resolve through the same knowledge filename detection uses: a word
 * that is already a Shiki ID wins, then the alias table, then the
 * extension map (rs → rust, py → python, js → javascript, sh → bash,
 * yml → yaml).
 *
 * @returns Shiki language identifier, or undefined if unknown
 */
export function languageFromInfoString(info: string | null | undefined): string | undefined {
  if (!info) return undefined;
  const word = info.trim().split(/\s+/)[0]?.toLowerCase();
  if (!word) return undefined;
  // Own-key checks (#2787): an authored info string like "constructor"
  // must not resolve through Object.prototype members.
  if (Object.hasOwn(LANG_IMPORTS, word)) return word;
  if (Object.hasOwn(INFO_STRING_ALIASES, word)) return INFO_STRING_ALIASES[word];
  if (Object.hasOwn(EXT_TO_LANG, word)) return EXT_TO_LANG[word];
  return undefined;
}

/**
 * Determine the highlight language for one code_block node (#1479).
 *
 * Priority:
 * 1. The node's `language` attribute (raw mode / manual override)
 * 2. The node's `params` attribute — the markdown codec stores the
 *    fence's info string there in rich/markdown mode. Skipped for
 *    htmlBlock nodes: their `params` is a provenance value ("html"),
 *    not an authored language.
 * 3. The filename-detected language (may be undefined)
 */
export function resolveCodeBlockLanguage(
  attrs: { language?: unknown; params?: unknown; htmlBlock?: unknown },
  detectedLang: string | undefined,
): string | undefined {
  if (typeof attrs.language === "string" && attrs.language) return attrs.language;
  if (!attrs.htmlBlock && typeof attrs.params === "string") {
    const fromInfo = languageFromInfoString(attrs.params);
    if (fromInfo) return fromInfo;
  }
  return detectedLang;
}

// ---------------------------------------------------------------------------
// ProseMirror Plugin
// ---------------------------------------------------------------------------

/**
 * Options for creating the highlight plugin.
 */
export interface HighlightPluginOptions {
  /** Filename for automatic language detection. */
  filename?: string;
  /** Whether to show line numbers (default: true). */
  lineNumbers?: boolean;
}

/**
 * Create a ProseMirror plugin for syntax highlighting and line numbers.
 *
 * The plugin:
 * 1. Detects language per code_block: `language` attr, then the codec's
 *    `params` info string (rich/markdown mode), then the filename
 * 2. Lazily loads the Shiki grammar for that language
 * 3. Applies inline decorations for syntax tokens
 * 4. Optionally adds widget decorations for line numbers
 *
 * @param options - Configuration options
 * @returns ProseMirror plugin instance
 */
export function createSyntaxHighlightPlugin(options: HighlightPluginOptions = {}): Plugin {
  const { filename, lineNumbers = true } = options;
  const detectedLang = filename ? detectLanguage(filename) : undefined;

  // Cached highlighter + parser for synchronous path after first load
  let cachedHighlighter: ShikiHighlighter | null = null;
  let cachedShikiParser: Parser | null = null;

  // Create the async Shiki parser.
  // prosemirror-highlight async protocol:
  //   - Return Promise<void> to signal "loading, call me again later"
  //   - Return Decoration[] for synchronous results
  //   - After promise resolves, plugin re-invokes the parser
  const shikiParser: Parser = (parserOptions) => {
    // Determine language: manual override (node attr) → auto-detected → undefined
    const lang = (parserOptions.language as string | null) ?? detectedLang;
    if (!lang) return [];

    // Fast synchronous path: highlighter + language already loaded
    if (cachedHighlighter && loadedLangs.has(lang)) {
      if (!cachedShikiParser) {
        cachedShikiParser = createParser(cachedHighlighter, {
          themes: { dark: "vitesse-black" },
          defaultColor: "dark",
        });
      }
      const loadedLangIds = cachedHighlighter.getLoadedLanguages() as string[];
      if (loadedLangIds.includes(lang)) {
        const result = cachedShikiParser(parserOptions);
        // createParser may also return Promise<void> if theme isn't ready
        if (result instanceof Promise) return result;
        return result;
      }
      return [];
    }

    // Slow async path: load highlighter + grammar, return Promise<void>
    // Plugin will re-call this parser after the promise resolves.
    const loadPromise = getHighlighter().then(async (highlighter) => {
      cachedHighlighter = highlighter;
      await ensureLanguage(highlighter, lang);
    });
    return loadPromise as Promise<void>;
  };

  // Wrap with line numbers if enabled
  const parser: Parser = lineNumbers ? withLineNumbers(shikiParser) : shikiParser;

  return createHighlightPlugin({
    parser,
    nodeTypes: ["code_block"],
    languageExtractor: (node) => resolveCodeBlockLanguage(node.attrs, detectedLang),
  });
}
