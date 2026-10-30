# Now what

> You installed mcpx and ran the loop. Each link below is a question,
> and every one of them is answered one level down — the level-2 pages
> mapped in [the level-2 page map](./level-2-page-map.md).

---

- **Where does config go?** One `mcpServers` object, a search path
  that runs from the working directory up to `/etc/mcpx/config.json`,
  and settings that can come from a file, the environment or a flag:
  [configuration](./configuration.md), and the map's
  `configuration` page (common, server and client config, env vars).
- **How do I call mcpx from my harness, over MCP, over HTTP?** The CLI
  is one surface of four — the daemon also speaks MCP (`/mcp`,
  `mcpx serve`) and plain HTTP (`/v1/*`, OpenAPI at
  `/v1/openapi.json`): [interfaces](./interfaces.md),
  [cli](./cli.md), and the map's `mcp` and `http-api` pages.
- **How do scripts actually work?** The script contract, named
  scripts, runtimes and what a script may touch:
  [scripts](./scripts.md), [exec](./exec.md), [runtimes](./runtimes.md).
- **How does the opencode plugin work?** It finds the daemon so your
  harness does not have to ship a binary:
  [plugins](./plugins.md) and [the opencode plugin](./opencode-plugin.md).
- **Something is wrong.** `mcpx doctor` first (it says what to do
  about each failure), `mcpx status` for the daemon and its servers,
  and the skeptical questions, answered: [FAQ](./faq.md).
- **Everything, in depth.** The long version of all of it — the
  narrative the overview was distilled from, and the book it feeds:
  [the story](./story.md) and [the compendium](./compendium.md).
