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

let highlighterPromise: Promise<ShikiHighlighter> | null = null;
const loadedLangs = new Set<string>();

/**
 * Get or create the shared Shiki highlighter instance.
 * Uses the JavaScript regex engine (no WASM) for broad compatibility.
 * Starts with no languages loaded — they are added on demand.
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
 * No-op if the language is already loaded or unknown to Shiki.
 */
async function ensureLanguage(highlighter: ShikiHighlighter, lang: string): Promise<void> {
  if (loadedLangs.has(lang)) return;

  try {
    // Dynamic import of the language grammar bundle
    const langModule = await import(`@shikijs/langs/${lang}`);
    // Shiki lang modules export default an array of grammar definitions
    const langDefs = Array.isArray(langModule.default) ? langModule.default : [langModule.default];
    await highlighter.loadLanguage(...langDefs);
    loadedLangs.add(lang);
  } catch {
    // Language not available in Shiki — silently ignore
    // Mark as loaded to avoid repeated failed imports
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

  // Create the async Shiki parser
  const shikiParser: Parser = (parserOptions) => {
    // Determine language: manual override (node attr) → auto-detected → undefined
    const lang = (parserOptions.language as string | null) ?? detectedLang;
    if (!lang) return [];

    // Return a promise — prosemirror-highlight handles async gracefully:
    // it will call the parser again once the promise resolves.
    return getHighlighter().then(async (highlighter) => {
      await ensureLanguage(highlighter, lang);

      // Check if the language actually loaded successfully
      const loadedLangIds = highlighter.getLoadedLanguages() as string[];
      if (!loadedLangIds.includes(lang)) return;

      // Create the actual parser and run it
      const parser = createParser(highlighter, { themes: { dark: "vitesse-black" }, defaultColor: "dark" });
      const decorations = parser(parserOptions);
      // If decorations are a Promise (shouldn't be after loading), return void
      if (decorations instanceof Promise) return;
      // Store decorations for the plugin to pick up on re-parse
      return decorations;
    }) as unknown as Promise<void>;
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
