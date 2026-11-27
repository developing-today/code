// #8570 Phase 3: read the projector projection store in-process. The
// configured HTTP ProjectorReader remains a fallback when the local store
// cannot be constructed or read.
import { createStoreProjectorReader, type ProjectorReader } from "./sources/projector.ts";

export interface PmCoreProjectorReaderEnv extends Record<string, string | undefined> {
  PM_CORE_PROJECTOR_DATABASE_URL?: string;
  PM_CORE_PROJECTOR_TURSO_AUTH_TOKEN?: string;
}

/**
 * Construct a store-backed reader when pm-core has been given the
 * projector database location. The import is lazy so standalone runs that
 * stay on the HTTP reader do not load projector runtime modules.
 */
export function createPmCoreProjectorReader(
  env: PmCoreProjectorReaderEnv = process.env,
  httpFallback?: ProjectorReader,
): ProjectorReader | null {
  const databaseUrl = env.PM_CORE_PROJECTOR_DATABASE_URL?.trim();
  if (!databaseUrl) return null;

  let readerPromise: Promise<ProjectorReader | null> | undefined;
  const reader = async (): Promise<ProjectorReader | null> => {
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
    }).catch(() => null);
    const resolved = await readerPromise;
    if (!resolved) readerPromise = undefined;
    return resolved;
  };

  const read = async <T>(
    fromStore: (reader: ProjectorReader) => Promise<T | null>,
    fromHttp: (reader: ProjectorReader) => Promise<T | null>,
  ): Promise<T | null> => {
    const storeReader = await reader();
    if (storeReader) {
      try {
        const result = await fromStore(storeReader);
        if (result !== null) return result;
      } catch {
        // A store failure degrades to the HTTP reader when it is configured.
      }
    }
    if (!httpFallback) return null;
    try {
      return await fromHttp(httpFallback);
    } catch {
      return null;
    }
  };

  return {
    items: (refs) => read((source) => source.items(refs), (source) => source.items(refs)),
    comments: (refs) => read((source) => source.comments(refs), (source) => source.comments(refs)),
    latestComments: (refs) =>
      read((source) => source.latestComments(refs), (source) => source.latestComments(refs)),
    links: (subject) => read((source) => source.links(subject), (source) => source.links(subject)),
  };
}
