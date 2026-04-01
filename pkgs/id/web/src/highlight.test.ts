/**
 * Tests for syntax highlighting and line number support.
 * Tests language detection, extension mapping, and plugin creation.
 */

import { describe, expect, it } from "vitest";
import { createSyntaxHighlightPlugin, detectLanguage } from "./highlight";

// ============================================================================
// Language Detection: Extension Mapping
// ============================================================================

describe("detectLanguage", () => {
  describe("JavaScript / TypeScript family", () => {
    it("detects .js as javascript", () => {
      expect(detectLanguage("app.js")).toBe("javascript");
    });

    it("detects .mjs as javascript", () => {
      expect(detectLanguage("module.mjs")).toBe("javascript");
    });

    it("detects .cjs as javascript", () => {
      expect(detectLanguage("config.cjs")).toBe("javascript");
    });

    it("detects .ts as typescript", () => {
      expect(detectLanguage("editor.ts")).toBe("typescript");
    });

    it("detects .mts as typescript", () => {
      expect(detectLanguage("utils.mts")).toBe("typescript");
    });

    it("detects .jsx as jsx", () => {
      expect(detectLanguage("Component.jsx")).toBe("jsx");
    });

    it("detects .tsx as tsx", () => {
      expect(detectLanguage("Component.tsx")).toBe("tsx");
    });
  });

  describe("systems languages", () => {
    it("detects .rs as rust", () => {
      expect(detectLanguage("main.rs")).toBe("rust");
    });

    it("detects .go as go", () => {
      expect(detectLanguage("main.go")).toBe("go");
    });

    it("detects .c as c", () => {
      expect(detectLanguage("main.c")).toBe("c");
    });

    it("detects .h as c", () => {
      expect(detectLanguage("header.h")).toBe("c");
    });

    it("detects .cpp as cpp", () => {
      expect(detectLanguage("main.cpp")).toBe("cpp");
    });

    it("detects .hpp as cpp", () => {
      expect(detectLanguage("header.hpp")).toBe("cpp");
    });

    it("detects .cs as csharp", () => {
      expect(detectLanguage("Program.cs")).toBe("csharp");
    });

    it("detects .swift as swift", () => {
      expect(detectLanguage("App.swift")).toBe("swift");
    });

    it("detects .kt as kotlin", () => {
      expect(detectLanguage("Main.kt")).toBe("kotlin");
    });

    it("detects .java as java", () => {
      expect(detectLanguage("Main.java")).toBe("java");
    });

    it("detects .scala as scala", () => {
      expect(detectLanguage("Main.scala")).toBe("scala");
    });

    it("detects .dart as dart", () => {
      expect(detectLanguage("main.dart")).toBe("dart");
    });
  });

  describe("scripting languages", () => {
    it("detects .py as python", () => {
      expect(detectLanguage("script.py")).toBe("python");
    });

    it("detects .rb as ruby", () => {
      expect(detectLanguage("app.rb")).toBe("ruby");
    });

    it("detects .php as php", () => {
      expect(detectLanguage("index.php")).toBe("php");
    });

    it("detects .pl as perl", () => {
      expect(detectLanguage("script.pl")).toBe("perl");
    });

    it("detects .sh as bash", () => {
      expect(detectLanguage("build.sh")).toBe("bash");
    });

    it("detects .bash as bash", () => {
      expect(detectLanguage("run.bash")).toBe("bash");
    });

    it("detects .zsh as bash", () => {
      expect(detectLanguage("config.zsh")).toBe("bash");
    });

    it("detects .fish as fish", () => {
      expect(detectLanguage("config.fish")).toBe("fish");
    });

    it("detects .ps1 as powershell", () => {
      expect(detectLanguage("script.ps1")).toBe("powershell");
    });

    it("detects .bat as bat", () => {
      expect(detectLanguage("run.bat")).toBe("bat");
    });

    it("detects .cmd as bat", () => {
      expect(detectLanguage("build.cmd")).toBe("bat");
    });

    it("detects .r as r", () => {
      expect(detectLanguage("analysis.r")).toBe("r");
    });
  });

  describe("web languages", () => {
    it("detects .html as html", () => {
      expect(detectLanguage("index.html")).toBe("html");
    });

    it("detects .htm as html", () => {
      expect(detectLanguage("page.htm")).toBe("html");
    });

    it("detects .css as css", () => {
      expect(detectLanguage("styles.css")).toBe("css");
    });

    it("detects .scss as scss", () => {
      expect(detectLanguage("theme.scss")).toBe("scss");
    });

    it("detects .sass as sass", () => {
      expect(detectLanguage("theme.sass")).toBe("sass");
    });

    it("detects .less as less", () => {
      expect(detectLanguage("styles.less")).toBe("less");
    });
  });

  describe("data / config formats", () => {
    it("detects .json as json", () => {
      expect(detectLanguage("package.json")).toBe("json");
    });

    it("detects .toml as toml", () => {
      expect(detectLanguage("Cargo.toml")).toBe("toml");
    });

    it("detects .yaml as yaml", () => {
      expect(detectLanguage("config.yaml")).toBe("yaml");
    });

    it("detects .yml as yaml", () => {
      expect(detectLanguage("ci.yml")).toBe("yaml");
    });

    it("detects .xml as xml", () => {
      expect(detectLanguage("pom.xml")).toBe("xml");
    });

    it("detects .ini as ini", () => {
      expect(detectLanguage("settings.ini")).toBe("ini");
    });

    it("detects .cfg as ini", () => {
      expect(detectLanguage("setup.cfg")).toBe("ini");
    });

    it("detects .conf as ini", () => {
      expect(detectLanguage("nginx.conf")).toBe("ini");
    });

    it("detects .cmake as cmake", () => {
      expect(detectLanguage("build.cmake")).toBe("cmake");
    });
  });

  describe("query / schema languages", () => {
    it("detects .sql as sql", () => {
      expect(detectLanguage("query.sql")).toBe("sql");
    });

    it("detects .graphql as graphql", () => {
      expect(detectLanguage("schema.graphql")).toBe("graphql");
    });

    it("detects .proto as protobuf", () => {
      expect(detectLanguage("service.proto")).toBe("protobuf");
    });
  });

  describe("other languages", () => {
    it("detects .md as markdown", () => {
      expect(detectLanguage("README.md")).toBe("markdown");
    });

    it("detects .nix as nix", () => {
      expect(detectLanguage("flake.nix")).toBe("nix");
    });

    it("detects .lua as lua", () => {
      expect(detectLanguage("init.lua")).toBe("lua");
    });

    it("detects .zig as zig", () => {
      expect(detectLanguage("main.zig")).toBe("zig");
    });

    it("detects .ex as elixir", () => {
      expect(detectLanguage("app.ex")).toBe("elixir");
    });

    it("detects .exs as elixir", () => {
      expect(detectLanguage("test_helper.exs")).toBe("elixir");
    });

    it("detects .erl as erlang", () => {
      expect(detectLanguage("server.erl")).toBe("erlang");
    });

    it("detects .hs as haskell", () => {
      expect(detectLanguage("Main.hs")).toBe("haskell");
    });

    it("detects .ml as ocaml", () => {
      expect(detectLanguage("main.ml")).toBe("ocaml");
    });

    it("detects .vim as viml", () => {
      expect(detectLanguage("plugin.vim")).toBe("viml");
    });

    it("detects .tf as hcl", () => {
      expect(detectLanguage("main.tf")).toBe("hcl");
    });

    it("detects .cts as typescript", () => {
      expect(detectLanguage("config.cts")).toBe("typescript");
    });

    it("detects .gradle as groovy", () => {
      expect(detectLanguage("build.gradle")).toBe("groovy");
    });

    it("detects .markdown as markdown", () => {
      expect(detectLanguage("notes.markdown")).toBe("markdown");
    });
  });

  // ==========================================================================
  // Special Filenames
  // ==========================================================================

  describe("special filenames", () => {
    it("detects Dockerfile as dockerfile", () => {
      expect(detectLanguage("Dockerfile")).toBe("dockerfile");
    });

    it("detects dockerfile (lowercase) as dockerfile", () => {
      expect(detectLanguage("dockerfile")).toBe("dockerfile");
    });

    it("detects Dockerfile.dev as dockerfile", () => {
      expect(detectLanguage("Dockerfile.dev")).toBe("dockerfile");
    });

    it("detects Makefile as makefile", () => {
      expect(detectLanguage("Makefile")).toBe("makefile");
    });

    it("detects makefile (lowercase) as makefile", () => {
      expect(detectLanguage("makefile")).toBe("makefile");
    });

    it("detects justfile as just", () => {
      expect(detectLanguage("justfile")).toBe("just");
    });

    it("detects Gemfile as ruby", () => {
      expect(detectLanguage("Gemfile")).toBe("ruby");
    });

    it("detects Rakefile as ruby", () => {
      expect(detectLanguage("Rakefile")).toBe("ruby");
    });

    it("detects CMakeLists.txt as cmake", () => {
      expect(detectLanguage("CMakeLists.txt")).toBe("cmake");
    });

    it("detects Vagrantfile as ruby", () => {
      expect(detectLanguage("Vagrantfile")).toBe("ruby");
    });
  });

  // ==========================================================================
  // Dotfiles
  // ==========================================================================

  describe("dotfiles", () => {
    it("detects .gitignore as gitignore", () => {
      expect(detectLanguage(".gitignore")).toBe("gitignore");
    });

    it("detects .dockerignore as gitignore", () => {
      expect(detectLanguage(".dockerignore")).toBe("gitignore");
    });

    it("detects .env as dotenv", () => {
      expect(detectLanguage(".env")).toBe("dotenv");
    });

    it("detects .env.local as dotenv", () => {
      expect(detectLanguage(".env.local")).toBe("dotenv");
    });

    it("detects .editorconfig as properties", () => {
      expect(detectLanguage(".editorconfig")).toBe("properties");
    });

    it("detects .prettierrc as json", () => {
      expect(detectLanguage(".prettierrc")).toBe("json");
    });

    it("detects .eslintrc as json", () => {
      expect(detectLanguage(".eslintrc")).toBe("json");
    });
  });

  // ==========================================================================
  // Path Handling
  // ==========================================================================

  describe("path handling", () => {
    it("extracts basename from path", () => {
      expect(detectLanguage("src/main.rs")).toBe("rust");
    });

    it("handles deeply nested paths", () => {
      expect(detectLanguage("packages/core/src/lib/utils.ts")).toBe("typescript");
    });

    it("handles special filenames with path", () => {
      expect(detectLanguage("docker/Dockerfile")).toBe("dockerfile");
    });
  });

  // ==========================================================================
  // Edge Cases
  // ==========================================================================

  describe("edge cases", () => {
    it("returns undefined for unknown extension", () => {
      expect(detectLanguage("file.xyz")).toBeUndefined();
    });

    it("returns undefined for extensionless file", () => {
      expect(detectLanguage("LICENSE")).toBeUndefined();
    });

    it("returns undefined for empty string", () => {
      expect(detectLanguage("")).toBeUndefined();
    });

    it("is case-insensitive for extensions", () => {
      expect(detectLanguage("App.JS")).toBe("javascript");
      expect(detectLanguage("style.CSS")).toBe("css");
      expect(detectLanguage("main.RS")).toBe("rust");
    });

    it("handles files with multiple dots", () => {
      expect(detectLanguage("config.test.ts")).toBe("typescript");
      expect(detectLanguage("my.component.tsx")).toBe("tsx");
    });

    it("handles .lock files (no highlighting)", () => {
      // .lock, .sum, .mod — not in EXT_TO_LANG, return undefined
      expect(detectLanguage("package-lock.json")).toBe("json");
      expect(detectLanguage("bun.lockb")).toBeUndefined();
    });
  });
});

// ============================================================================
// Plugin Creation
// ============================================================================

describe("createSyntaxHighlightPlugin", () => {
  it("returns a plugin object", () => {
    const plugin = createSyntaxHighlightPlugin();
    expect(plugin).toBeDefined();
    // ProseMirror plugins have a spec property
    expect(plugin.spec).toBeDefined();
  });

  it("accepts filename option", () => {
    const plugin = createSyntaxHighlightPlugin({ filename: "main.rs" });
    expect(plugin).toBeDefined();
    expect(plugin.spec).toBeDefined();
  });

  it("accepts lineNumbers option", () => {
    const plugin = createSyntaxHighlightPlugin({ lineNumbers: false });
    expect(plugin).toBeDefined();
    expect(plugin.spec).toBeDefined();
  });

  it("accepts all options together", () => {
    const plugin = createSyntaxHighlightPlugin({
      filename: "editor.ts",
      lineNumbers: true,
    });
    expect(plugin).toBeDefined();
    expect(plugin.spec).toBeDefined();
  });
});
