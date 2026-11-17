# mcpx — for agents

mcpx is code mode for MCP: every MCP server behind one daemon, driven
by scripts instead of schemas. This page is the agent twin of the mcpx
home page. Every other page of the book has a twin beside it at
`<page>.html.md`; `llms.txt` indexes them all and `llms-full.txt`
concatenates levels 1–2.

## Install mcpx (one-shot procedure)

You are an agent setting mcpx up on this machine. Do these steps in
order; stop and report if a step fails.

1. **Install the latest mcpx.** The channel that exists today is nix:

   ```sh
   nix profile install github:developing-today/code#mcpx
   ```

   (No nix? The other channels — container, release installer — and
   their current status are on the install page: `install.html.md`.
   Do not invent a download URL.)

2. **Verify — the version guard.** Run:

   ```sh
   mcpx --version
   mcpx doctor
   ```

   `mcpx doctor` diagnoses the installation and says what to do about
   each failed check. The docs inside the binary always match the
   installed version: if anything on this page or any twin disagrees
   with `mcpx help`, prefer `mcpx help` — that preference is the
   version guard, and it is why this prompt never pins a version.

3. **Wire mcpx into your harness.**

   - **opencode** (full wiring): copy the plugin from the repository
     checkout of developing-today/code:

     ```sh
     mkdir -p ~/.config/opencode/plugin
     cp -R pkgs/mcpx/plugin/opencode/mcpx-session.ts pkgs/mcpx/plugin/opencode/mcpx ~/.config/opencode/plugin/
     ```

     `mcpx/` must stay a subdirectory of `plugin/`. The plugin finds
     the daemon over its unix socket and needs no mcpx binary on its
     own machine, but a daemon must be running somewhere; enable its
     tools with `MCPX_PLUGIN_TOOLS=1` (or `tools: true` in the plugin
     config) and its guidance with `instructions: true`.

   - **Any other harness:** run mcpx as an MCP server. Point the
     harness's MCP configuration at mcpx (the daemon autostarts on
     first use); per-harness adapters beyond "run mcpx as an MCP
     server" are not written yet.

4. **Confirm the loop works:**

   ```sh
   mcpx ls
   ```

   Then drive servers with the loop — `ls` / `search` / `types` /
   `run` | `exec` — as `AGENTS.md` in the repository and the twins of
   the CLI and exec pages (`cli.html.md`, `exec.html.md`) describe.
