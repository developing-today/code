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
        getItems: (refs, options) => store.getItems(db, refs, options),
        getComments: (refs, options) => store.getComments(db, refs, options),
        getLinks: (options) => store.getLinks(db, options),
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
