// Stamping seams for the create path (#12585; design §3.1, §5).
//
// The stamping endpoint draws fleet-owned provenance from three
// service-side sources besides the session registry (src/sessions.ts)
// and the validated token: the queue record (for queue-originated
// work), the GitHub writer (src/github.ts — the hosted rotation
// point), and the metadata DB the create path inserts into. Each is
// an interface here or in github.ts, with a fixture implementation —
// the TenancyDirectory / SessionRegistry idiom — because the
// production homes are other issues' deliverables:
//
//   - QueueRecordSource: the queue front door (#12589) serves claims
//     from the one queue DB (design §6); until it lands there is no
//     hosted claim record to read, so the Worker wires no source and
//     queue fields stamp null — the design §5 rule for work that is
//     not queue-originated. A test (or #12589) injects a source
//     behind this interface without touching the route.
//   - MetadataStore: D1MetadataStore (#15456) persists the complete
//     record JSON and extracted keys behind this interface when
//     QUEUE_D1 is bound; the fixture is the local/dev fallback.
//     Insert failure semantics live in the
//     route: a failed insert never loses the GitHub item — ingestion
//     (#12590, design §7.3) heals it.

/** The queue record's stamp-relevant facts (design §5 table), read
 * by the service from the queue DB — never from caller assertion. */
export interface QueueRecord {
  queueId: string;
  /** The queue packet's scheduled start (ISO 8601), when it has one. */
  scheduledStart: string | null;
  /** The claim record's pickup time (ISO 8601). */
  pickedUpAt: string | null;
  /** The claim/bind record's work-start time (ISO 8601). */
  workStartedAt: string | null;
  /** The queue record's lane-start issue set, when it carries one. */
  laneStartIssues: string[];
}

export interface QueueRecordSource {
  /** The queue record bound to this caller's session, or null when
   * the work is not queue-originated. */
  recordForSession(bindingId: string, sessionId: string): Promise<QueueRecord | null>;
}

/** Fixture queue source: an in-memory map keyed by session id, with
 * the same binding check the registry applies — a record recorded
 * against another tenancy answers null, never the record. */
export class FixtureQueueRecordSource implements QueueRecordSource {
  private readonly records = new Map<string, { bindingId: string; record: QueueRecord }>();

  seed(bindingId: string, sessionId: string, record: QueueRecord): void {
    this.records.set(sessionId, { bindingId, record: { ...record, laneStartIssues: [...record.laneStartIssues] } });
  }

  async recordForSession(bindingId: string, sessionId: string): Promise<QueueRecord | null> {
    const entry = this.records.get(sessionId);
    if (!entry || entry.bindingId !== bindingId) return null;
    return { ...entry.record, laneStartIssues: [...entry.record.laneStartIssues] };
  }
}

/** One metadata DB row (#9074's shape as the create path writes it):
 * the full Metadata JSON plus the extracted keys provenance queries
 * filter by (#9060 §15). */
export interface MetadataRecord {
  repo: string;
  kind: "issue" | "pr";
  number: number;
  url: string;
  /** The tenancy binding_id whose caller created the item. */
  bindingId: string;
  /** The full Metadata block, exactly as stamped into the body. */
  metadata: Record<string, unknown>;
  /** Extracted keys (#9060 §15): direct parent, session, lane, host,
   * queue id, and the links' relation/status pairs. */
  extracted: {
    directParent: unknown;
    sessionId: string | null;
    lane: string | null;
    host: string | null;
    queueId: string | null;
    links: { target: unknown; relation: unknown; status: unknown }[];
  };
  /** Service-clock insert time (ISO 8601). */
  insertedAt: string;
}

export interface MetadataStore {
  insert(record: MetadataRecord): Promise<void>;
  get(repo: string, kind: "issue" | "pr", number: number): Promise<MetadataRecord | null>;
  /** Every row belonging to one tenancy that matches the filter
   * (#12587; design §7.1). Tenancy scoping lives in the store, not
   * the route: a store can never be asked for another account's
   * rows. A Turso-backed store pushes the filter onto its indexed
   * extracted keys (#9060 §15); the fixture matches in memory with
   * the same matcher the query surface applies to GitHub-fallback
   * candidates, so both sources answer a filter identically. */
  list(bindingId: string, filter: MetadataFilter): Promise<MetadataRecord[]>;
  /** Every row stored for one repo, across tenancies (#14312).
   * The divergence report's operator seam: orphan detection must
   * see rows whose item has vanished from GitHub, including rows
   * no tenancy list could attribute. This does NOT widen the
   * query door — its per-tenancy `list` rule above stands; this
   * seam is for the ingestion door's operator surfaces (backfill
   * runs, divergence reports), which are service surfaces, not
   * per-tenancy reads. */
  listRepo(repo: string): Promise<MetadataRecord[]>;
}

/** A Metadata query filter (#12587; #9060 §13 query path, design
 * §7.1): list/filter by Metadata fields — direct parent, session,
 * lane, host, queue id, link relation/status pairs. Every field is
 * optional; a set field must match (AND across fields). The link
 * relation and status, when both are set, must hold of the SAME
 * link — they are a pair, not two independent existentials. */
export interface MetadataFilter {
  repo?: string;
  kind?: "issue" | "pr";
  directParent?: { repo: string; number: number };
  sessionId?: string;
  lane?: string;
  host?: string;
  queueId?: string;
  linkRelation?: string;
  linkStatus?: string;
  linkTarget?: { repo: string; number: number };
}

/** Does a stored record's extracted key set name the same
 * {repo, number} item reference? (§12 references are objects; a
 * non-object value never matches a reference filter.) */
function sameItemRef(value: unknown, ref: { repo: string; number: number }): boolean {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  return v["repo"] === ref.repo && v["number"] === ref.number;
}

/** The one matcher for Metadata filters, applied to a record's
 * extracted keys. The fixture store and the query surface's GitHub
 * fallback share it, so a filter means the same thing over both
 * sources (#9060 §13: one query path, two sources). */
export function matchesMetadataFilter(record: MetadataRecord, filter: MetadataFilter): boolean {
  if (filter.repo !== undefined && record.repo !== filter.repo) return false;
  if (filter.kind !== undefined && record.kind !== filter.kind) return false;
  const x = record.extracted;
  if (filter.directParent !== undefined && !sameItemRef(x.directParent, filter.directParent)) {
    return false;
  }
  if (filter.sessionId !== undefined && x.sessionId !== filter.sessionId) return false;
  if (filter.lane !== undefined && x.lane !== filter.lane) return false;
  if (filter.host !== undefined && x.host !== filter.host) return false;
  if (filter.queueId !== undefined && x.queueId !== filter.queueId) return false;
  if (filter.linkRelation !== undefined || filter.linkStatus !== undefined || filter.linkTarget !== undefined) {
    const hit = x.links.some(
      (link) =>
        (filter.linkRelation === undefined || link.relation === filter.linkRelation) &&
        (filter.linkStatus === undefined || link.status === filter.linkStatus) &&
        (filter.linkTarget === undefined || sameItemRef(link.target, filter.linkTarget)),
    );
    if (!hit) return false;
  }
  return true;
}

function cloneRecord(record: MetadataRecord): MetadataRecord {
  return JSON.parse(JSON.stringify(record)) as MetadataRecord;
}

/** Fixture metadata store for the skeleton seam (see the header
 * note). Records are cloned in and out so callers cannot mutate the
 * store's rows. `failNextInsert` is the forced-failure knob the
 * acceptance's insert-failure case drives. */
export class FixtureMetadataStore implements MetadataStore {
  private readonly rows = new Map<string, MetadataRecord>();
  failNextInsert = false;

  private key(repo: string, kind: string, number: number): string {
    return `${repo}#${kind}#${number}`;
  }

  async insert(record: MetadataRecord): Promise<void> {
    if (this.failNextInsert) {
      this.failNextInsert = false;
      throw new Error("fixture metadata store: forced insert failure");
    }
    this.rows.set(this.key(record.repo, record.kind, record.number), cloneRecord(record));
  }

  async get(repo: string, kind: "issue" | "pr", number: number): Promise<MetadataRecord | null> {
    const row = this.rows.get(this.key(repo, kind, number));
    return row ? cloneRecord(row) : null;
  }

  async list(bindingId: string, filter: MetadataFilter): Promise<MetadataRecord[]> {
    const out: MetadataRecord[] = [];
    for (const row of this.rows.values()) {
      if (row.bindingId !== bindingId) continue;
      if (!matchesMetadataFilter(row, filter)) continue;
      out.push(cloneRecord(row));
    }
    return out;
  }

  async listRepo(repo: string): Promise<MetadataRecord[]> {
    const out: MetadataRecord[] = [];
    for (const row of this.rows.values()) {
      if (row.repo !== repo) continue;
      out.push(cloneRecord(row));
    }
    return out;
  }
}

/** The §15 extracted keys for a stamped block. */
export function extractKeys(metadata: Record<string, unknown>): MetadataRecord["extracted"] {
  const provenance = (metadata["provenance"] ?? {}) as Record<string, unknown>;
  const links = Array.isArray(metadata["links"]) ? metadata["links"] : [];
  return {
    directParent: metadata["direct_parent"] ?? null,
    sessionId: typeof provenance["session_id"] === "string" ? provenance["session_id"] : null,
    lane: typeof provenance["lane"] === "string" ? provenance["lane"] : null,
    host: typeof provenance["host"] === "string" ? provenance["host"] : null,
    queueId: typeof provenance["queue_id"] === "string" ? provenance["queue_id"] : null,
    links: links.map((link) => {
      const l = (link ?? {}) as Record<string, unknown>;
      return { target: l["target"] ?? null, relation: l["relation"] ?? null, status: l["status"] ?? null };
    }),
  };
}
