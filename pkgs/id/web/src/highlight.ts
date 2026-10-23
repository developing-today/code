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
 * override support via code_block node's `language` attribute.
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
const EXT_TO_LANG: Record<string, string> = {
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
const FILENAME_TO_LANG: Record<string, string> = {
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
 * 1. Special filename match (case-insensitive, ignoring extension)
 * 2. Extension match from EXT_TO_LANG map
 * 3. undefined (no highlighting)
 *
 * @param filename - The filename (may include path components)
 * @returns Shiki language identifier, or undefined if unknown
 */
export function detectLanguage(filename: string): string | undefined {
  // Extract basename (handle paths)
  const basename = filename.split("/").pop() ?? filename;

  // Check special filenames (case-insensitive, without extension)
  const basenameLower = basename.toLowerCase();
  const nameWithoutExt = basenameLower.includes(".")
    ? basenameLower.slice(0, basenameLower.lastIndexOf("."))
    : basenameLower;

  // Try full basename first (e.g., "Dockerfile.dev" → still a Dockerfile)
  if (FILENAME_TO_LANG[nameWithoutExt]) {
    return FILENAME_TO_LANG[nameWithoutExt];
  }
  // Try full basename without extension check (e.g., "Dockerfile" with no ext)
  if (FILENAME_TO_LANG[basenameLower]) {
    return FILENAME_TO_LANG[basenameLower];
  }

  // Dotfiles: .gitignore, .env, .editorconfig, .prettierrc, .eslintrc
  if (basenameLower.startsWith(".")) {
    const dotName = basenameLower.slice(1);
    if (dotName === "gitignore" || dotName === "dockerignore") return "gitignore";
    if (dotName === "env" || dotName.startsWith("env.")) return "dotenv";
    if (dotName === "editorconfig") return "properties";
    // .prettierrc, .eslintrc — usually JSON
    if (dotName === "prettierrc" || dotName === "eslintrc") return "json";
  }

  // Extract extension
  const ext = basename.includes(".") ? basename.split(".").pop()?.toLowerCase() : undefined;
  if (!ext) return undefined;

  return EXT_TO_LANG[ext];
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
 */
const LANG_IMPORTS: Record<string, () => Promise<any>> = {
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
  html: () => import("@shikijs/langs/html"),
  css: () => import("@shikijs/langs/css"),
  scss: () => import("@shikijs/langs/scss"),
  json: () => import("@shikijs/langs/json"),
  yaml: () => import("@shikijs/langs/yaml"),
  toml: () => import("@shikijs/langs/toml"),
  xml: () => import("@shikijs/langs/xml"),
  sql: () => import("@shikijs/langs/sql"),
  graphql: () => import("@shikijs/langs/graphql"),
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

  const importFn = LANG_IMPORTS[lang];
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
 * 1. Detects language from filename or code_block `language` attribute
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
    languageExtractor: (node) => {
      // Manual override via node attribute takes priority
      const nodeLanguage = node.attrs.language as string | undefined;
      if (nodeLanguage) return nodeLanguage;
      // Fall back to filename-based detection
      return detectedLang;
    },
  });
}
