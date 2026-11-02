/**
 * End-to-end highlight tests for the 12 grammars registered for #749.
 *
 * The registry tests in highlight.test.ts prove a grammar import
 * function exists for every detected language; these tests prove the
 * grammars actually load through createSyntaxHighlightPlugin and
 * produce token decorations in a live editor — the acceptance
 * criterion that was unmet when #749's fix (PR #750) landed with
 * registration tests only.
 */

import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { afterEach, describe, expect, it, vi } from "vitest";
import { rawSchema } from "./editor";
import { createSyntaxHighlightPlugin, detectLanguage } from "./highlight";

const CASES: { lang: string; filename: string; sample: string }[] = [
  { lang: "bat", filename: "run.bat", sample: "@echo off\nset NAME=world\necho %NAME%\n" },
  { lang: "sass", filename: "theme.sass", sample: "$color: red\n.button\n  color: $color\n" },
  { lang: "less", filename: "styles.less", sample: "@width: 10px;\n.box { width: @width; }\n" },
  {
    lang: "protobuf",
    filename: "service.proto",
    sample: 'syntax = "proto3";\nmessage User {\n  string name = 1;\n}\n',
  },
  { lang: "zig", filename: "main.zig", sample: 'const std = @import("std");\npub fn main() void {}\n' },
  { lang: "erlang", filename: "server.erl", sample: "-module(hello).\n-export([world/0]).\nworld() -> ok.\n" },
  { lang: "ocaml", filename: "main.ml", sample: "let square x = x * x\nlet () = print_int (square 3)\n" },
  { lang: "viml", filename: "plugin.vim", sample: "set number\nlet g:enabled = 1\n" },
  { lang: "hcl", filename: "main.tf", sample: 'resource "aws_s3_bucket" "b" {\n  bucket = "name"\n}\n' },
  { lang: "just", filename: "justfile", sample: "build:\n    cargo build\n" },
  { lang: "dotenv", filename: ".env", sample: "API_KEY=abc123\nDEBUG=true\n" },
  { lang: "properties", filename: ".editorconfig", sample: "root = true\n[*]\nindent_style = space\n" },
];

const views: EditorView[] = [];

afterEach(() => {
  for (const view of views.splice(0)) {
    view.destroy();
  }
  document.body.innerHTML = "";
});

/** Inline decoration styles currently applied by the highlight plugin. */
function decorationStyles(plugin: ReturnType<typeof createSyntaxHighlightPlugin>, view: EditorView): string[] {
  const state = plugin.getState(view.state) as
    | { decorations?: { find: () => { type: { attrs?: Record<string, string> } }[] } }
    | undefined;
  const decorations = state?.decorations?.find() ?? [];
  return decorations
    .map((d) => d.type.attrs?.style)
    .filter((s): s is string => typeof s === "string" && s.length > 0);
}

describe("end-to-end highlighting for the #749 grammars", () => {
  for (const { lang, filename, sample } of CASES) {
    it(`highlights ${filename} as ${lang} through the plugin`, async () => {
      expect(detectLanguage(filename)).toBe(lang);

      const plugin = createSyntaxHighlightPlugin({ filename, lineNumbers: false });
      const doc = rawSchema.node("doc", null, [
        rawSchema.node("code_block", null, [rawSchema.text(sample)]),
      ]);
      const state = EditorState.create({ schema: rawSchema, doc, plugins: [plugin] });
      const container = document.createElement("div");
      document.body.appendChild(container);
      const view = new EditorView(container, { state });
      views.push(view);

      // The grammar loads asynchronously: the parser first returns a
      // promise, the plugin re-renders once it resolves. Wait for real
      // token decorations, then require more than one token colour —
      // a grammar that failed to load leaves the block undecorated,
      // and a plain-text fallback could never produce several styles.
      await vi.waitFor(
        () => {
          expect(decorationStyles(plugin, view).length).toBeGreaterThan(0);
        },
        { timeout: 15000, interval: 50 },
      );
      const styles = decorationStyles(plugin, view);
      expect(new Set(styles).size).toBeGreaterThanOrEqual(2);
    });
  }
});
