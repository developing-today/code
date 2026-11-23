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
//   - MetadataStore: the metadata DB is #9074's deliverable (design
//     §3.1: "the create path inserts the full record into the
//     metadata database"); a Turso-backed store replaces the fixture
//     behind this interface. Insert failure semantics live in the
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
