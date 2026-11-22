# id-mail Worker publishing

The `id-mail-worker-publish` workflow uploads a new version of the existing
`mail` Worker after changes land on `main`. It never creates a deployment,
changes routes or custom domains, retires another Worker, or writes Worker
secrets. A successful workflow run therefore means only that a version was
uploaded; the active Worker keeps serving its current deployment.

The upload uses the pinned `cf` CLI and the Workers Versions API. Its
`cloudflare.config.ts` declares the current `MAIL_CONFIG`, `EMAIL`, and
`LIMITER` version bindings and the existing SQLite `Limiter` class. The live
`Limiter` export represents the current `v1` SQLite migration in the `cf`
configuration model. It leaves routes and `workers.dev` triggers undeclared
because version upload does not apply trigger changes. The existing `CALLERS`
secret is neither read nor written. The config declares `CALLERS` as a
required secret binding without providing its value. Wrangler is pinned only
as the build backend used internally by `cf`; the publisher invokes
`cf workers versions create`. Verify the `CALLERS` binding on an uploaded
version before promoting it.

## Cutover prerequisites

- Configure the repository variable `CLOUDFLARE_ACCOUNT_ID` and repository
  secret `CLOUDFLARE_API_TOKEN`. The token must be account-scoped and limited
  to Workers Scripts read/write. Do not use the broad account-wide token.
- Keep this workflow limited to pushes on `main`; it intentionally has no pull
  request or manual dispatch trigger, and grants only `contents: read`.
- Confirm that the `mail` Worker already exists. This publisher is not a
  bootstrap path and does not create resources.
- Before a separate, reviewed promotion, inspect the uploaded version and
  confirm its `CALLERS`, `EMAIL`, `LIMITER`, and `MAIL_CONFIG` bindings and the
  existing Durable Object class are present. Then promote that version through
  an explicit Workers deployment operation. For example:

  ```sh
  cf workers deployments create --worker mail --strategy percentage \
    --versions '[{"version_id":"<VERSION_ID>","percentage":100}]'
  ```

  Promotion is outside this workflow.
- Review ownership of the existing `id-mail` Worker and its routes separately.
  The `retire` list in `config.json` is not acted on by this publisher.
