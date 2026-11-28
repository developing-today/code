import type { DashComment, DashAuthor, SessionRecord } from "./model.ts";

/** One owner-confirmed association; exact author fields prevent fuzzy matches. */
export interface OwnerConfirmedNameLink {
  author: Pick<DashAuthor, "kind" | "id" | "label">;
  identity: Pick<SessionRecord, "source" | "id">;
}

/**
 * Apply only explicitly supplied owner confirmations. No name discovery,
 * normalization, author rewrite, or provenance promotion occurs here.
 */
export function applyOwnerConfirmedNameLinks(
  comments: DashComment[],
  registry: SessionRecord[],
  confirmations: OwnerConfirmedNameLink[],
): { comments: DashComment[]; linked: number } {
  const available = new Set(registry.map((record) => `${record.source}\0${record.id}`));
  const byAuthor = new Map<string, OwnerConfirmedNameLink>();
  const ambiguousAuthors = new Set<string>();
  for (const confirmation of confirmations) {
    const author = confirmation.author;
    const key = JSON.stringify([author.kind, author.id, author.label]);
    if (!available.has(`${confirmation.identity.source}\0${confirmation.identity.id}`)) continue;
    if (byAuthor.has(key)) {
      byAuthor.delete(key);
      ambiguousAuthors.add(key);
      continue;
    }
    if (ambiguousAuthors.has(key)) continue;
    byAuthor.set(key, confirmation);
  }

  let linked = 0;
  const migrated = comments.map((comment) => {
    if (comment.provenance === "verified" || comment.linkedIdentity) return comment;
    const match = byAuthor.get(JSON.stringify([comment.author.kind, comment.author.id, comment.author.label]));
    if (!match) return comment;
    linked++;
    return {
      ...comment,
      linkedIdentity: { source: match.identity.source, id: match.identity.id },
    };
  });
  return { comments: migrated, linked };
}
