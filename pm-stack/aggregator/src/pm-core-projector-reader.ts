// #8570 Phase 3: read the projector projection store in-process. The
// HTTP ProjectorReader remains the fallback when no database URL is set;
// pm-core opts into this adapter for its shared projector store.
import { createStoreProjectorReader, type ProjectorReader } from "./sources/projector.ts";

export interface PmCoreProjectorReaderEnv extends Record<string, string | undefined> {
  PM_CORE_PROJECTOR_DATABASE_URL?: string;
  PM_CORE_PROJECTOR_TURSO_AUTH_TOKEN?: string;
}

/**
 * Construct a store-backed reader when pm-core has been given the
 * projector database location. The import is lazy so ordinary aggregator
 * installs do not need the projector runtime unless the in-process path
 * is configured.
 */
export function createPmCoreProjectorReader(
  env: PmCoreProjectorReaderEnv = process.env,
): ProjectorReader | null {
  const databaseUrl = env.PM_CORE_PROJECTOR_DATABASE_URL?.trim();
  if (!databaseUrl) return null;

  let readerPromise: Promise<ProjectorReader> | undefined;
  const reader = async (): Promise<ProjectorReader> => {
    readerPromise ??= import("../../projector/src/store.ts").then((store) => {
      const db = store.openDb({
        DATABASE_URL: databaseUrl,
        TURSO_AUTH_TOKEN: env.PM_CORE_PROJECTOR_TURSO_AUTH_TOKEN,
      });
      return createStoreProjectorReader({
        async getItems(refs, options) {
          return (await store.getItems(db, refs, options)).map((row) => ({
            ref: row.ref,
            repo: row.repo,
            number: row.number,
            kind: row.kind,
            url: row.url,
            title: row.title,
            state: row.state,
            state_reason: row.state_reason,
            draft: row.draft ?? undefined,
            merged: row.merged ?? undefined,
            author: row.author,
            labels: row.labels,
            assignees: row.assignees,
            created_at: row.created_at,
            updated_at: row.updated_at,
            closed_at: row.closed_at,
            merged_at: row.merged_at,
          }));
        },
        async getComments(refs, options) {
          return (await store.getComments(db, refs, options)).map((row) => ({
            id: row.id,
            ref: row.ref,
            kind: row.kind,
            author: row.author,
            body: row.body,
            created_at: row.created_at,
            url: row.url ?? undefined,
            path: row.path,
            line: row.line,
            review_state: row.review_state,
            commit_id: row.commit_id,
            deleted: row.deleted,
          }));
        },
        async getLinks(options) {
          return (await store.getLinks(db, options)).map((row) => ({
            subject: row.subject,
            ref: row.ref,
            relation: row.relation,
            note: row.note,
            created_by: row.created_by,
            created_at: row.created_at,
          }));
        },
      });
    });
    return readerPromise;
  };

  return {
    async items(refs) {
      return (await reader()).items(refs);
    },
    async comments(refs) {
      return (await reader()).comments(refs);
    },
    async latestComments(refs) {
      return (await reader()).latestComments(refs);
    },
    async links(subject) {
      return (await reader()).links(subject);
    },
  };
}
