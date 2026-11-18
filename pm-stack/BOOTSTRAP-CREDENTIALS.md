# Bootstrap credentials — pm-stack cold restore

Names only. This page never records a credential value, token file content,
or account recovery detail. Every value is Drew-held or lives in Bitwarden
Secrets Manager (BWS).

The roots of trust sit outside every backup set. A new computer cannot
restore from the backups alone, so it bootstraps in this order before
step 2 of `COLD-RESTORE-RUNBOOK.md`:

1. **BWS machine-account token.** Drew signs into Bitwarden and mints a
   BWS machine-account token. The token is not stored in BWS, by design.
   Keep it out of the repo and out of backups.
2. **Restic passwords and R2/Supabase secrets.** With the BWS token, fetch
   by name:
   - `restic.repo-archive-password`
   - `restic.sandbox-config-password`
   - `restic.platform-binaries-password`
   - the R2 and Supabase secrets for the backup and restore legs
3. **rclone Google Drive remote.** Drew signs into Google interactively so
   rclone's `drive` remote gets a fresh OAuth token. This cannot be
   automated and is not stored in any backup.
4. **1Password service-account tokens.** Fetch from BWS by the `op muse…`
   names.
5. **SOPS age key.** Tracked in its own gap issue. Reference that issue;
   do not solve it on this page.

**Sandbox scheduler re-creation** is a platform provisioning step, not a
backup restore. Do it by the platform procedure, not from the backups.
