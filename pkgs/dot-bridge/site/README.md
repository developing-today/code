# ChatGPT connector Site

The private Site's Worker is now the ChatGPT side of the message bridge:
after Sites signs the caller in, it passes each `/mcp` request to the
bridge's `/mcp/<chatgpt token>` (see `worker/index.js`), so ChatGPT runs
the same tools as Claude over the same store. Set the bridge's ChatGPT
token (Bitwarden `message-bridge.chatgpt-token`) as the Site secret
`MESSAGE_BRIDGE_TOKEN`, or, where the Site has no secrets, upload it as
the R2 object `config/message-bridge-token` in `BUCKET`. Optional:
`MESSAGE_BRIDGE_URL` (default `https://bridge.security.cab`) and
`OWNER_USER_ID` (only that Sites user). The packaging below is unchanged.

Run `bash scripts/test-helpers.sh` for hermetic build and staging checks. Run `node test.mjs` to test the Worker with synthetic identity/storage.
For a local artifact, set `SITE_PROJECT_ID` and run `npm run build` and
`npm run validate`. Artifact creation does not push or publish anything.

## Source sync and release for an authorized private Site

1. Confirm the target private Site and obtain its native source write access.
   Open a separate Site checkout with the Sites workflow helper:
   `node <sites-plugin-root>/scripts/site-workflow.mjs --project-id <id>`.
   Pass the returned native credential in workflow JSON over hidden stdin
   after the ready prompt, never in argv or a file. Keep the opening result.
2. In that checkout, use the native Site setup to establish its exact identity,
   Worker configuration, `BUCKET` R2 binding and `mcp` capability. Stage
   the allowlisted source locally with
   `bash scripts/prepare-site-source.sh "$SITE_PROJECT_ID" /absolute/native-checkout`
   **from this repo's Site directory**. The helper refuses a mismatched
   manifest and never writes that manifest itself.
3. Review changes. For authorized native source push/packaging, run the Sites
   workflow helper again for that same project. Pass its prior opening result
   as `source`, the authorized `credential` over hidden stdin, an absolute
   `archivePath`, and ordered `commands`: `["node","test.mjs"]`,
   `["env","SITE_PROJECT_ID=<id>","bash","scripts/build.sh"]`, and
   `["node","scripts/validate-artifact.mjs"]`. This native helper commits,
   pushes, and packages; it does not publish. Keep its verified receipt.
4. Publication is separate. For confirmed owner-private targets use the native
   private save/deploy action or save version then deploy private version,
   using the helper's exact project/commit/archive tuple. Keep access private.
   Owner publication/installation and ordinary ChatGPT acceptance are distinct
   gates; do not infer them from successful local tests or a Dot-only check.

Real messages now flow through this Worker; keep the Site private.
