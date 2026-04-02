Use Context7 (`context7_resolve-library-id` then `context7_query-docs`) to look up library and framework **documentation** — API usage, examples, guides, and up-to-date references. Prefer Context7 over guessing APIs or relying on training data for external libraries.

## When to use Context7 vs codedb_remote

- **Context7**: Documentation, API references, usage guides, code examples, "how do I use X?"
- **`codedb_remote`**: Raw source code of a known repo, implementation internals, "how does X work under the hood?"

If you need actual source code from a library's repo, prefer `codedb_remote` — it's faster and gives you the real code. Use Context7 for curated documentation and examples.

## Retry limits

Do not call either Context7 tool more than 3 times per question. After 3 failed or unhelpful attempts, STOP retrying and switch strategy: use `webfetch` to fetch the library's official docs site directly (e.g. `https://docs.example.com/api`), then use what you learn to refine a final Context7 query if needed. A web fetch can also help you discover the correct library name or ID before querying Context7.

## Authentication

Context7 uses `CONTEXT7_API_KEY` from the environment for authenticated access (higher rate limits). This is loaded via `.env.local` through direnv — do not hardcode it in config files.
