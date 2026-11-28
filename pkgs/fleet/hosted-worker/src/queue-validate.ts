// Packet validation for the queue door (#12589) — a faithful
// TypeScript port of the native broker's validator
// (pkgs/fleet/crates/fleet-queue/src/validate.rs), which is itself
// the contract of record for repo-task/v1 packets: the same checks,
// the same error/warning strings, the same accept/reject semantics,
// so a packet means the same thing at both front doors (design §6:
// the TS door's validator is fixture-checked against the same
// contract — here it is transcribed from the native validator and
// pinned by the golden cases in test/queue.test.ts, which are drawn
// from validate.rs's own test corpus).
//
// The validation contract (native design #6448): the queue validates
// every value it forwards — type, range, and agreement with the
// packet's other fields — at the boundary, before the task row
// exists. Reject, never clamp, never warn-and-forward; warnings keep
// their existing role (design defaults, size targets) and never
// carry a validity verdict.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type JsonObject = { [key: string]: Json };

export function isObj(v: unknown): v is JsonObject {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** serde_json Value::as_i64: an integer-valued number in i64 range. */
export function asI64(v: unknown): number | null {
  return typeof v === "number" && Number.isInteger(v) && Number.isSafeInteger(v) ? v : null;
}

/** serde_json Value::as_u64 (for max_attempts): a non-negative integer. */
function asU64(v: unknown): number | null {
  const n = asI64(v);
  return n !== null && n >= 0 ? n : null;
}

function get(obj: JsonObject, field: string): Json | undefined {
  return obj[field];
}

// ---------------------------------------------------------------------------
// Time: RFC 3339 <-> epoch, the native crate's civil-date math
// (validate.rs parse_rfc3339_epoch; no Date.parse — its acceptance
// set differs, e.g. it normalizes impossible dates the native
// parser refuses).
// ---------------------------------------------------------------------------

function daysFromCivil(y: number, m: number, d: number): number {
  const yAdj = m <= 2 ? y - 1 : y;
  const era = Math.floor((yAdj >= 0 ? yAdj : yAdj - 399) / 400);
  const yoe = yAdj - era * 400; // [0, 399]
  const mp = m > 2 ? m - 3 : m + 9; // [0, 11]
  const doy = Math.floor((153 * mp + 2) / 5) + d - 1; // [0, 365]
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy; // [0, 146096]
  return era * 146097 + doe - 719468;
}

function civilFromDays(z0: number): [number, number, number] {
  const z = z0 + 719468;
  const era = Math.floor((z >= 0 ? z : z - 146096) / 146097);
  const doe = z - era * 146097; // [0, 146096]
  const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365); // [0, 399]
  const y = yoe + era * 400;
  const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100)); // [0, 365]
  const mp = Math.floor((5 * doy + 2) / 153); // [0, 11]
  const d = doy - Math.floor((153 * mp + 2) / 5) + 1; // [1, 31]
  const m = mp < 10 ? mp + 3 : mp - 9; // [1, 12]
  return [m <= 2 ? y + 1 : y, m, d];
}

/** Format an epoch second as RFC 3339 UTC — the native `iso()`
 * shape (second precision, Z suffix), whose lexicographic order is
 * chronological order; every stored timestamp uses it. */
export function iso(epochSeconds: number): string {
  const days = Math.floor(epochSeconds / 86400);
  const secs = epochSeconds - days * 86400;
  const [y, m, d] = civilFromDays(days);
  const p2 = (n: number) => String(n).padStart(2, "0");
  return `${String(y).padStart(4, "0")}-${p2(m)}-${p2(d)}T${p2(Math.floor(secs / 3600))}:${p2(
    Math.floor((secs % 3600) / 60),
  )}:${p2(secs % 60)}Z`;
}

/** Parse an RFC 3339 timestamp to epoch seconds, or null. Mirrors
 * validate.rs's parse_rfc3339_epoch exactly: fixed field widths,
 * real calendar validation (the day must exist in the month/year),
 * validated offset and fraction components. */
export function parseRfc3339Epoch(input: string): number | null {
  const s = input.trim();
  const splitAt = (sep: string): [string, string] | null => {
    const idx = s.indexOf(sep);
    return idx < 0 ? null : [s.slice(0, idx), s.slice(idx + 1)];
  };
  const dateTime = splitAt("T") ?? splitAt("t") ?? splitAt(" ");
  if (!dateTime) return null;
  const [date, rest] = dateTime;
  const dp = date.split("-");
  if (dp.length !== 3) return null;
  const digits = (v: string, n: number): number | null => (v.length === n && /^[0-9]+$/.test(v) ? Number(v) : null);
  const year = digits(dp[0], 4);
  const month = digits(dp[1], 2);
  const day = digits(dp[2], 2);
  if (year === null || month === null || day === null) return null;
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  const daysInMonth = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
  if (daysInMonth === undefined || day < 1 || day > daysInMonth) return null;

  let time: string;
  let offsetS = 0;
  if (rest.endsWith("Z") || rest.endsWith("z")) {
    time = rest.slice(0, -1);
  } else {
    const idx = Math.max(rest.lastIndexOf("+"), rest.lastIndexOf("-"));
    if (idx > 0) {
      const off = rest.slice(idx + 1);
      const sign = rest[idx] === "+" ? 1 : -1;
      const op = off.split(":");
      if (op.length > 2) return null;
      const oh = digits(op[0], 2);
      const om = op.length > 1 ? digits(op[1], 2) : 0;
      if (oh === null || om === null) return null;
      if (oh > 23 || om > 59) return null;
      offsetS = sign * (oh * 3600 + om * 60);
      time = rest.slice(0, idx);
    } else if (idx === 0) {
      return null;
    } else {
      time = rest;
    }
  }
  let fraction: string | null = null;
  const dot = time.indexOf(".");
  if (dot >= 0) {
    fraction = time.slice(dot + 1);
    time = time.slice(0, dot);
    if (fraction === "" || !/^[0-9]+$/.test(fraction)) return null;
  }
  const tp = time.split(":");
  if (tp.length < 2 || tp.length > 3) return null;
  const hh = digits(tp[0], 2);
  const mm = digits(tp[1], 2);
  const ss = tp.length > 2 ? digits(tp[2], 2) : 0;
  if (hh === null || mm === null || ss === null) return null;
  if (hh >= 24 || mm >= 60 || ss > 60) return null;
  return daysFromCivil(year, month, day) * 86400 + hh * 3600 + mm * 60 + ss - offsetS;
}

/** The not_before horizon (#6426): a not_before more than 366 days
 * in the future is REJECTED, never clamped. */
export const NOT_BEFORE_MAX_FUTURE_SECONDS = 366 * 86400;

export function validateNotBeforeAt(value: Json, field: string, now: number): string | null {
  if (typeof value !== "string") {
    const kind =
      typeof value === "number"
        ? "a number"
        : typeof value === "boolean"
          ? "a boolean"
          : Array.isArray(value)
            ? "an array"
            : value === null
              ? "null"
              : "an object";
    return `${field}: must be an RFC 3339 timestamp string, got ${kind}`;
  }
  if (value.trim() === "") {
    return `${field}: must not be empty (an RFC 3339 timestamp is required)`;
  }
  const epoch = parseRfc3339Epoch(value);
  if (epoch === null) {
    return `${field}: '${value}' is not a valid RFC 3339 timestamp`;
  }
  if (epoch - now > NOT_BEFORE_MAX_FUTURE_SECONDS) {
    return `${field}: ${value} is more than 366 days in the future — a task gated that far out is born unclaimable (#6426); rejected, not clamped`;
  }
  return null;
}

// ---------------------------------------------------------------------------
// The validator proper (validate.rs, transcribed).
// ---------------------------------------------------------------------------

export const PRIORITY_SCALE = [10, 20, 30, 40, 50, 70, 90];
export const EXPEDITE_PRIORITY = 15;
export const NEXT_UP_PRIORITY = 25;
export const KINDS = [
  "pr_shepherd",
  "triage",
  "small_fix",
  "ci_investigation",
  "second_pass",
  "feature_slice",
  "prompt",
];
export const FLEET_REPOS = [
  "developing-today/code",
  "developing-today/opencode",
  "developing-today/openchamber",
  "developing-today-forks/opencode",
  "developing-today-forks/openchamber",
  "dezren39/nix",
  "dezren39/darwin-nix",
  "developing-today/zettel",
  "dezren39/dezren39",
];
const PAYLOAD_WARN_BYTES = 4096;
const PAYLOAD_MAX_BYTES = 16384;
const BODY_WARN_WORDS = 250;
export const MAX_ATTEMPTS_CEILING = 1000;
export const TASK_DEFAULT_MAX_ATTEMPTS = 1000;

export const DEDUPE_PATTERNS_DOC =
  "fix: repo-health:v1:fix:<repo>:issue:<n>\n" +
  "triage: repo-health:v1:triage:<repo>:issue:<n>\n" +
  "dispatch: fix/triage issue key may append:dispatch:<8-or-16-hex>\n" +
  "pr: repo-health:v1:pr:<repo>:pr:<n>\n" +
  "pr-set: repo-health:v1:pr-set:<repo>:<n>-<n>[-...] (sorted)\n" +
  "set: repo-health:v1:set:<origin issue>:owed:<repo>\n" +
  "maintenance: repo-health:v1:maintenance:<repo>:<kind>:<YYYY-Www>\n" +
  "second-pass: repo-health:v1:second-pass:<repo>:<issue>:pass-<n>\n" +
  "slice: repo-health:v1:slice:<repo>:issue:<n>:slice-<k>\n" +
  "prompt: repo-health:v1:prompt:<repo>:submission:<UUIDv4>";

/** Rust's `{:?}` debug rendering of a list (strings quoted, ", "
 * separators) — the shape several validator messages embed. */
export function debugList(values: (string | number)[]): string {
  return `[${values.map((v) => (typeof v === "string" ? JSON.stringify(v) : String(v))).join(", ")}]`;
}

/** Python-style repr for the one message family that embeds a value. */
function pythonRepr(v: Json | undefined): string {
  if (v === undefined || v === null) return "None";
  if (typeof v === "string") return `'${v}'`;
  return JSON.stringify(v);
}

function validMaxAttempts(value: Json): boolean {
  const n = asU64(value);
  return n !== null && n >= 1 && n <= MAX_ATTEMPTS_CEILING;
}

function isPositiveInt(v: Json | undefined): boolean {
  const n = v === undefined ? null : asI64(v);
  return n !== null && n >= 1;
}

function manualSchemaErrors(p: Json): string[] {
  const errs: string[] = [];
  if (!isObj(p)) return ["payload: must be a JSON object"];
  const obj = p;
  if (get(obj, "schema") !== "repo-task/v1") {
    errs.push("payload.schema: must be 'repo-task/v1'");
  }
  const kind = get(obj, "kind");
  if (!(typeof kind === "string" && KINDS.includes(kind))) {
    errs.push(`payload.kind: must be one of ${debugList([...KINDS].sort())}`);
  }
  if (get(obj, "kind") === "prompt") {
    const prompt = get(obj, "prompt");
    const id = isObj(prompt) ? get(prompt, "id") : undefined;
    if (
      typeof id !== "string" ||
      !/^prompt:[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id)
    ) {
      errs.push("payload.prompt.id: must be a canonical prompt:<UUIDv4>");
    }
  }
  const repo = get(obj, "repo");
  if (!(typeof repo === "string" && FLEET_REPOS.includes(repo))) {
    errs.push(`payload.repo: ${pythonRepr(repo)} is not a fleet repo`);
  }
  const flags = isObj(get(obj, "risk")) ? get(obj["risk"] as JsonObject, "flags") : undefined;
  if (
    get(obj, "implementation_repo") === undefined &&
    Array.isArray(flags) &&
    flags.some((f) => f === "cross-repo-change")
  ) {
    errs.push(
      "payload.implementation_repo: required for cross-repo-change; repair the explicit implementation target before claiming",
    );
  }
  const target = get(obj, "implementation_repo");
  if (target !== undefined) {
    const valid =
      typeof target === "string" &&
      (() => {
        const parts = target.split("/");
        return (
          parts.length === 2 &&
          parts.every(
            (part) => part !== "" && part !== "." && part !== ".." && [...part].every((c) => /[A-Za-z0-9._-]/.test(c)),
          )
        );
      })();
    if (!valid) {
      errs.push("payload.implementation_repo: must be an owner/name repository");
    }
  }
  for (const f of ["problem", "priority_reason"]) {
    const v = get(obj, f);
    if (typeof v === "string" && v !== "") {
      // ok
    } else if (typeof v === "string" || v === undefined) {
      errs.push(`payload.${f}: required field missing or empty`);
    } else {
      errs.push(`payload.${f}: must be a non-empty string`);
    }
  }
  for (const f of ["evidence", "acceptance", "tests", "context_refs"]) {
    const v = get(obj, f);
    if (Array.isArray(v)) {
      if (v.some((e) => typeof e !== "string")) {
        errs.push(`payload.${f}: every element must be a string`);
      }
    } else {
      errs.push(`payload.${f}: must be a list`);
    }
  }
  for (const f of ["evidence", "acceptance", "tests"]) {
    const v = get(obj, f);
    if (Array.isArray(v) && v.length === 0) {
      errs.push(`payload.${f}: must not be empty`);
    }
  }
  const scope = get(obj, "scope");
  if (isObj(scope)) {
    const include = get(scope, "include");
    if (Array.isArray(include)) {
      if (include.some((e) => typeof e !== "string")) {
        errs.push("payload.scope.include: every element must be a string");
      }
    } else {
      errs.push("payload.scope.include: must be a list");
    }
    const exclude = get(scope, "exclude");
    if (Array.isArray(exclude) && exclude.length > 0) {
      if (exclude.some((e) => typeof e !== "string")) {
        errs.push("payload.scope.exclude: every element must be a string");
      }
    } else {
      errs.push("payload.scope.exclude: required (scope exclusions)");
    }
  } else {
    errs.push("payload.scope: must be an object with include/exclude");
  }
  const risk = get(obj, "risk");
  const riskLevel = isObj(risk) ? get(risk, "level") : undefined;
  if (isObj(risk) && typeof riskLevel === "string" && ["low", "medium", "high"].includes(riskLevel)) {
    const rflags = get(risk, "flags");
    if (Array.isArray(rflags)) {
      if (rflags.some((e) => typeof e !== "string")) {
        errs.push("payload.risk.flags: every element must be a string");
      }
    } else {
      errs.push("payload.risk.flags: must be a list");
    }
  } else {
    errs.push("payload.risk.level: must be low|medium|high");
  }
  if (!isPositiveInt(get(obj, "timebox_minutes"))) {
    errs.push("payload.timebox_minutes: must be a positive integer");
  }
  const rbm = get(obj, "required_budget_minutes");
  if (rbm !== undefined && rbm !== null && !isPositiveInt(rbm)) {
    errs.push("payload.required_budget_minutes: must be a positive integer");
  }
  const pma = get(obj, "max_attempts");
  if (pma !== undefined && !validMaxAttempts(pma)) {
    errs.push(`payload.max_attempts: must be an integer between 1 and ${MAX_ATTEMPTS_CEILING}`);
  }
  const dedupe = get(obj, "dedupe");
  const dedOk =
    isObj(dedupe) &&
    typeof get(dedupe, "checked_at") === "string" &&
    (get(dedupe, "checked_at") as string) !== "" &&
    typeof get(dedupe, "basis") === "string" &&
    (get(dedupe, "basis") as string) !== "";
  if (!dedOk) {
    errs.push("payload.dedupe: must have checked_at and basis as non-empty strings");
  }
  const issue = get(obj, "issue");
  if (isObj(issue)) {
    if (!isPositiveInt(get(issue, "number"))) {
      errs.push("payload.issue.number: must be a positive integer");
    }
  }
  const hasIssue = isObj(issue) && isPositiveInt(get(issue, "number"));
  const prs = get(obj, "prs");
  const hasPrs =
    Array.isArray(prs) && prs.length > 0 && prs.every((el) => isObj(el) && isPositiveInt(get(el, "number")));
  const set = get(obj, "set");
  const hasSet = isObj(set) && Object.keys(set).length > 0;
  const source = get(obj, "source");
  const hasSource =
    isObj(source) &&
    isPositiveInt(get(source, "pr")) &&
    typeof get(source, "finding") === "string" &&
    (get(source, "finding") as string) !== "";
  const prompt = get(obj, "prompt");
  const promptId = isObj(prompt) ? get(prompt, "id") : undefined;
  const hasPromptIdentity =
    get(obj, "kind") === "prompt" &&
    typeof promptId === "string" &&
    /^prompt:[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(promptId);
  if (!(hasIssue || hasPrs || hasSet || hasSource || hasPromptIdentity)) {
    errs.push(
      "payload identity: need issue.number, a non-empty prs list, a set object, or a pre-filing source {pr, finding}",
    );
  }
  errs.push(...reviewErrors(obj));
  errs.push(...sliceErrors(obj));
  return errs;
}

function reviewErrors(p: JsonObject): string[] {
  const errs: string[] = [];
  const kind = typeof get(p, "kind") === "string" ? (get(p, "kind") as string) : "";
  const review = get(p, "review");
  if (kind !== "second_pass" && review === undefined) return errs;
  if (!isObj(review)) {
    errs.push("payload.review: required object for kind second_pass (design #2025 section 5.2)");
    return errs;
  }
  const url = get(review, "verdict_note_url");
  if (!(typeof url === "string" && url !== "")) {
    errs.push("payload.review.verdict_note_url: required non-empty string for second_pass packets");
  }
  const gapList = get(review, "gap_list");
  if (!(Array.isArray(gapList) && gapList.length > 0 && gapList.every((g) => typeof g === "string" && g !== ""))) {
    errs.push("payload.review.gap_list: required non-empty list of checkable gap strings for second_pass packets");
  }
  const pass = asI64(get(review, "pass") ?? null);
  const passCap = asI64(get(review, "pass_cap") ?? null);
  if (!(pass !== null && pass >= 1)) {
    errs.push(
      "payload.review.pass: required positive integer (the pass being commissioned; the original work is pass 0)",
    );
  }
  if (!(passCap !== null && passCap >= 1)) {
    errs.push("payload.review.pass_cap: required positive integer");
  }
  if (pass !== null && passCap !== null && pass > passCap) {
    errs.push(
      `payload.review.pass: pass ${pass} exceeds pass_cap ${passCap} — the termination cap is enforced at enqueue (design #2025 section 5.1)`,
    );
  }
  const verdict = get(review, "verdict");
  if (verdict !== undefined && typeof verdict !== "string") {
    errs.push("payload.review.verdict: must be a string");
  }
  const artifact = get(review, "artifact");
  if (artifact !== undefined && !isObj(artifact)) {
    errs.push("payload.review.artifact: must be an object {pr, issue, merged}");
  }
  const producer = get(review, "producer");
  if (producer !== undefined && !isObj(producer)) {
    errs.push("payload.review.producer: must be an object {task_id, session, pr}");
  }
  const refs = get(review, "goal_context_refs");
  if (refs !== undefined && !(Array.isArray(refs) && refs.every((r) => typeof r === "string"))) {
    errs.push("payload.review.goal_context_refs: must be a list of strings");
  }
  const floor = get(review, "route_floor");
  if (floor !== undefined && typeof floor !== "string") {
    errs.push("payload.review.route_floor: must be a string");
  }
  return errs;
}

function sliceErrors(p: JsonObject): string[] {
  const errs: string[] = [];
  const kind = typeof get(p, "kind") === "string" ? (get(p, "kind") as string) : "";
  const slice = get(p, "slice");
  if (kind !== "feature_slice" && slice === undefined) return errs;
  if (!isObj(slice)) {
    errs.push("payload.slice: required object for kind feature_slice (#12233 F2c)");
    return errs;
  }
  let parent: number | null = null;
  if (isPositiveInt(get(slice, "parent_issue"))) {
    parent = asI64(get(slice, "parent_issue") ?? null);
  } else {
    errs.push("payload.slice.parent_issue: must be a positive integer");
  }
  if (!isPositiveInt(get(slice, "number"))) {
    errs.push("payload.slice.number: must be a positive integer");
  }
  const issue = get(p, "issue");
  const ino = isObj(issue) ? asI64(get(issue, "number") ?? null) : null;
  if (parent !== null && ino !== null && parent !== ino) {
    errs.push(`payload.slice.parent_issue: ${parent} does not match payload.issue.number ${ino}`);
  }
  for (const f of ["criterion", "done_check"]) {
    const v = get(slice, f);
    if (!(typeof v === "string" && v !== "")) {
      errs.push(`payload.slice.${f}: required non-empty string for feature_slice packets`);
    }
  }
  const owned = get(slice, "owned_paths");
  if (!(Array.isArray(owned) && owned.length > 0 && owned.every((s) => typeof s === "string" && s !== ""))) {
    errs.push("payload.slice.owned_paths: required non-empty list of non-empty path strings for feature_slice packets");
  }
  return errs;
}

// --- Dedupe-key families (match_dedupe / dedupe_errors, transcribed).

export interface DedupeMatch {
  family: string;
  repo: string;
  num: number | null;
  nums: number[] | null;
  origin: number | null;
  pass: number | null;
}

function allDigits(s: string): boolean {
  return s !== "" && /^[0-9]+$/.test(s);
}

/** Canonical dedupe numbers only: plain decimal digits, no leading
 * zeros, at least 1 (#5510). */
function canonicalNumber(s: string): number | null {
  if (!allDigits(s)) return null;
  const n = Number(s);
  if (!Number.isSafeInteger(n) || n < 1 || String(n) !== s) return null;
  return n;
}

function issueNumberWithDispatchSuffix(tail: string): number | null {
  const idx = tail.indexOf(":dispatch:");
  let number = tail;
  let suffix: string | null = null;
  if (idx >= 0) {
    number = tail.slice(0, idx);
    suffix = tail.slice(idx + ":dispatch:".length);
  }
  if (suffix !== null) {
    if (!(suffix.length === 8 || suffix.length === 16) || !/^[0-9a-fA-F]+$/.test(suffix)) {
      return null;
    }
  }
  return canonicalNumber(number);
}

export function matchDedupe(key: string): DedupeMatch | null {
  const PREFIX = "repo-health:v1:";
  if (!key.startsWith(PREFIX)) return null;
  const rest = key.slice(PREFIX.length);
  if (rest.startsWith("set:")) {
    const tail = rest.slice("set:".length);
    const idx = tail.indexOf(":owed:");
    if (idx >= 0) {
      const origin = canonicalNumber(tail.slice(0, idx));
      const repo = tail.slice(idx + ":owed:".length);
      if (origin !== null && FLEET_REPOS.includes(repo)) {
        return { family: "set", repo, num: null, nums: null, origin, pass: null };
      }
    }
    return null;
  }
  for (const fam of ["fix", "triage", "pr-set", "pr", "maintenance", "second-pass", "slice", "prompt"]) {
    if (!rest.startsWith(`${fam}:`)) continue;
    const after = rest.slice(fam.length + 1);
    for (const repo of FLEET_REPOS) {
      if (!after.startsWith(`${repo}:`)) continue;
      const tail = after.slice(repo.length + 1);
      if (fam === "prompt") {
        const prefix = "submission:";
        const id = tail.startsWith(prefix) ? tail.slice(prefix.length) : "";
        if (/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id)) {
          return { family: fam, repo, num: null, nums: null, origin: null, pass: null };
        }
      } else if (fam === "fix" || fam === "triage") {
        if (tail.startsWith("issue:")) {
          const num = issueNumberWithDispatchSuffix(tail.slice("issue:".length));
          if (num !== null) return { family: fam, repo, num, nums: null, origin: null, pass: null };
        }
      } else if (fam === "second-pass") {
        const idx = tail.indexOf(":pass-");
        if (idx >= 0) {
          const num = canonicalNumber(tail.slice(0, idx));
          const pass = canonicalNumber(tail.slice(idx + ":pass-".length));
          if (num !== null && pass !== null) {
            return { family: fam, repo, num, nums: null, origin: null, pass };
          }
        }
      } else if (fam === "slice") {
        if (tail.startsWith("issue:")) {
          const inner = tail.slice("issue:".length);
          const idx = inner.indexOf(":slice-");
          if (idx >= 0) {
            const num = canonicalNumber(inner.slice(0, idx));
            const sliceNo = canonicalNumber(inner.slice(idx + ":slice-".length));
            if (num !== null && sliceNo !== null) {
              return { family: fam, repo, num, nums: null, origin: null, pass: sliceNo };
            }
          }
        }
      } else if (fam === "pr") {
        if (tail.startsWith("pr:")) {
          const num = canonicalNumber(tail.slice("pr:".length));
          if (num !== null) return { family: fam, repo, num, nums: null, origin: null, pass: null };
        }
      } else if (fam === "pr-set") {
        const parts = tail.split("-");
        const nums = parts.map((part) => canonicalNumber(part));
        if (
          parts.length >= 2 &&
          nums.every((n) => n !== null) &&
          nums.every((n, i) => i === 0 || (nums[i - 1] as number) < (n as number))
        ) {
          return {
            family: fam,
            repo,
            num: null,
            nums: nums as number[],
            origin: null,
            pass: null,
          };
        }
      } else if (fam === "maintenance") {
        const idx = tail.lastIndexOf(":");
        if (idx >= 0) {
          const slug = tail.slice(0, idx);
          const week = tail.slice(idx + 1);
          const slugOk = slug !== "" && /^[a-z0-9]$/.test(slug[0]) && [...slug].every((c) => /[a-z0-9_-]/.test(c));
          const wb = [...week];
          const weekOk =
            week.length === 8 &&
            /^[0-9]{4}$/.test(week.slice(0, 4)) &&
            wb[4] === "-" &&
            wb[5] === "W" &&
            /^[0-9]{2}$/.test(week.slice(6)) &&
            Number(week.slice(6)) >= 1 &&
            Number(week.slice(6)) <= 53;
          if (slugOk && weekOk) {
            return { family: fam, repo, num: null, nums: null, origin: null, pass: null };
          }
        }
      }
    }
  }
  return null;
}

function kindFamilies(kind: string): string[] | null {
  switch (kind) {
    case "small_fix":
      return ["fix"];
    case "triage":
      return ["triage", "maintenance"];
    case "pr_shepherd":
      return ["pr", "pr-set", "set"];
    case "ci_investigation":
      return ["fix", "triage", "pr"];
    case "second_pass":
      return ["second-pass"];
    case "feature_slice":
      return ["slice"];
    case "prompt":
      return ["prompt"];
    default:
      return null;
  }
}

function dedupeErrors(packet: JsonObject, payload: JsonObject): string[] {
  const errors: string[] = [];
  const keyRaw = get(packet, "dedupe_key");
  if (!(typeof keyRaw === "string" && keyRaw !== "")) {
    return [`packet.dedupe_key: required deterministic key:\n${DEDUPE_PATTERNS_DOC}`];
  }
  const key = keyRaw;
  const m = matchDedupe(key);
  if (!m) {
    return [`packet.dedupe_key: '${key}' matches no design pattern:\n${DEDUPE_PATTERNS_DOC}`];
  }
  const payloadRepo = get(payload, "repo");
  if (payloadRepo !== undefined && payloadRepo !== m.repo) {
    errors.push(`packet.dedupe_key: repo '${m.repo}' in key does not match payload.repo ${pythonRepr(payloadRepo)}`);
  }
  const kind = typeof get(payload, "kind") === "string" ? (get(payload, "kind") as string) : "";
  const allowed = kindFamilies(kind);
  if (allowed && !allowed.includes(m.family)) {
    errors.push(
      `packet.dedupe_key: family '${m.family}' is not valid for kind '${kind}' (allowed: ${debugList([...allowed].sort())})`,
    );
  }
  const issue = get(payload, "issue");
  const issueNo = isObj(issue) ? asI64(get(issue, "number") ?? null) : null;
  const prsRaw = get(payload, "prs");
  const prNumbers: number[] = Array.isArray(prsRaw)
    ? prsRaw.map((el) => (isObj(el) ? asI64(get(el, "number") ?? null) : null)).filter((n): n is number => n !== null)
    : [];
  const source = get(payload, "source");
  const sourcePr = isObj(source) ? asI64(get(source, "pr") ?? null) : null;
  const set = get(payload, "set");
  const hasSet = isObj(set) && Object.keys(set).length > 0;
  const sourceOnly = sourcePr !== null && issueNo === null && prNumbers.length === 0 && !hasSet;

  if (m.family === "prompt") {
    const prompt = get(payload, "prompt");
    const id = isObj(prompt) ? get(prompt, "id") : undefined;
    const expected = typeof id === "string" ? id.replace(/^prompt:/, "") : "";
    if (key !== `repo-health:v1:prompt:${m.repo}:submission:${expected}`) {
      errors.push("payload.prompt.id: must match packet.dedupe_key canonical submission identity");
    }
    return errors;
  }

  if (m.family === "fix" || m.family === "triage" || m.family === "second-pass" || m.family === "slice") {
    const n = m.num ?? 0;
    if (issueNo !== null && n !== issueNo) {
      errors.push(`packet.dedupe_key: issue ${n} in key does not match payload.issue.number ${issueNo}`);
    } else if (issueNo === null) {
      errors.push(
        `packet.dedupe_key: family '${m.family}' requires payload.issue.number ${n} matching the key issue, but the payload carries no issue.number (#6446)`,
      );
    }
    if (m.family === "second-pass") {
      const keyPass = m.pass ?? 0;
      const review = get(payload, "review");
      const reviewPass = isObj(review) ? asI64(get(review, "pass") ?? null) : null;
      if (reviewPass !== null && reviewPass !== keyPass) {
        errors.push(`packet.dedupe_key: pass ${keyPass} in key does not match payload.review.pass ${reviewPass}`);
      } else if (reviewPass === null) {
        errors.push(
          `packet.dedupe_key: family 'second-pass' requires payload.review.pass ${keyPass} matching the key pass, but the payload carries no review.pass`,
        );
      }
    }
    if (m.family === "slice") {
      const keySlice = m.pass ?? 0;
      const slice = get(payload, "slice");
      const sliceNo = isObj(slice) ? asI64(get(slice, "number") ?? null) : null;
      if (sliceNo !== null && sliceNo !== keySlice) {
        errors.push(`packet.dedupe_key: slice ${keySlice} in key does not match payload.slice.number ${sliceNo}`);
      } else if (sliceNo === null) {
        errors.push(
          `packet.dedupe_key: family 'slice' requires payload.slice.number ${keySlice} matching the key slice, but the payload carries no slice.number`,
        );
      }
    }
  } else if (m.family === "pr") {
    const n = m.num ?? 0;
    if (prNumbers.length > 0) {
      if (!prNumbers.includes(n)) {
        errors.push(`packet.dedupe_key: PR ${n} in key is not in payload.prs ${debugList(prNumbers)}`);
      }
    } else if (sourceOnly) {
      const spr = sourcePr ?? 0;
      if (spr !== n) {
        errors.push(`packet.dedupe_key: PR ${n} in key does not match payload.source.pr ${spr}`);
      }
    } else {
      errors.push(
        `packet.dedupe_key: family 'pr' requires a non-empty payload.prs containing PR ${n} or a source-only payload.source.pr ${n}, but neither is present (#6446)`,
      );
    }
  } else if (m.family === "pr-set") {
    const nums = m.nums ?? [];
    const sorted = [...nums].sort((a, b) => a - b);
    if (nums.some((n, i) => n !== sorted[i])) {
      errors.push("packet.dedupe_key: pr-set numbers must be sorted ascending");
    }
    if (prNumbers.length > 0) {
      const sp = [...prNumbers].sort((a, b) => a - b);
      if (nums.length !== sp.length || nums.some((n, i) => n !== sp[i])) {
        errors.push(`packet.dedupe_key: pr-set ${debugList(nums)} does not match payload.prs ${debugList(sp)}`);
      }
    } else if (sourceOnly) {
      const spr = sourcePr ?? 0;
      if (!nums.includes(spr)) {
        errors.push(`packet.dedupe_key: pr-set ${debugList(nums)} in key does not contain payload.source.pr ${spr}`);
      }
    } else {
      errors.push(
        `packet.dedupe_key: family 'pr-set' requires a non-empty payload.prs matching pr-set ${debugList(nums)} or a source-only payload.source.pr in ${debugList(nums)}, but neither is present (#6446)`,
      );
    }
  } else if (m.family === "set") {
    const origin = m.origin ?? 0;
    const setOrigin = isObj(set) ? asI64(get(set, "origin_issue") ?? null) : null;
    if (setOrigin !== null && setOrigin !== origin) {
      errors.push(`packet.dedupe_key: origin issue ${origin} does not match payload.set.origin_issue ${setOrigin}`);
    } else if (setOrigin === null) {
      errors.push(
        `packet.dedupe_key: family 'set' requires payload.set.origin_issue ${origin} matching the key origin, but the payload carries no set.origin_issue (#6446)`,
      );
    }
  }
  return errors;
}

/** validate_packet: returns [errors, warnings]. `now` is epoch
 * seconds (the native reads the wall clock; the door injects its
 * service clock so tests control the horizon checks). */
export function validatePacket(packet: Json, now: number): [string[], string[]] {
  const errors: string[] = [];
  const warnings: string[] = [];
  if (!isObj(packet)) {
    return [["packet: must be a JSON object"], warnings];
  }
  if (get(packet, "manual_only") !== undefined && typeof get(packet, "manual_only") !== "boolean") {
    errors.push("packet.manual_only: must be a boolean");
  }
  const payload = get(packet, "payload");
  if (!isObj(payload)) {
    return [["packet.payload: required object (repo-task/v1 payload)"], warnings];
  }
  errors.push(...manualSchemaErrors(payload));

  const packetMaxAttempts = get(packet, "max_attempts");
  if (packetMaxAttempts !== undefined && !validMaxAttempts(packetMaxAttempts)) {
    errors.push(`packet.max_attempts: must be an integer between 1 and ${MAX_ATTEMPTS_CEILING}`);
  }
  const packetRbm = get(packet, "required_budget_minutes");
  if (packetRbm !== undefined && packetRbm !== null && !isPositiveInt(packetRbm)) {
    errors.push("packet.required_budget_minutes: must be a positive integer");
  }

  const title = get(packet, "title");
  if (!(typeof title === "string" && title.trim() !== "")) {
    errors.push("packet.title: required non-empty string");
  }
  const priority = get(packet, "priority");
  if (priority === undefined) {
    errors.push("packet.priority: required (numeric priority from the design scale: 10, 20, 30, 40, 50, 70, 90)");
  } else if (asI64(priority) !== null) {
    const v = asI64(priority) as number;
    if (!PRIORITY_SCALE.includes(v)) {
      errors.push(`packet.priority: ${v} is not on the design scale ${debugList(PRIORITY_SCALE)}`);
    }
  } else {
    errors.push("packet.priority: must be an integer");
  }
  const nb = get(packet, "not_before");
  if (nb !== undefined && nb !== null) {
    const err = validateNotBeforeAt(nb, "packet.not_before", now);
    if (err) errors.push(err);
  }
  const body = get(packet, "body");
  if (typeof body === "string" && body.trim() !== "") {
    const words = body.split(/\s+/).filter((w) => w !== "").length;
    if (words > BODY_WARN_WORDS) {
      warnings.push(`packet.body: ${words} words exceeds the ~${BODY_WARN_WORDS}-word design target`);
    }
  } else if (body !== undefined && body !== null) {
    errors.push("packet.body: must be a non-empty string");
  }
  const size = new TextEncoder().encode(JSON.stringify(payload)).length;
  if (size > PAYLOAD_MAX_BYTES) {
    errors.push(
      `payload size: ${size} bytes exceeds the ${PAYLOAD_MAX_BYTES}-byte hard ceiling (evidence and pointers, not full diffs/logs)`,
    );
  } else if (size > PAYLOAD_WARN_BYTES) {
    warnings.push(`payload size: ${size} bytes is above the ${PAYLOAD_WARN_BYTES}-byte design target`);
  }
  const tb = asI64(get(payload, "timebox_minutes") ?? null);
  if (tb !== null && tb !== 20 && tb !== 45) {
    warnings.push(`payload.timebox_minutes: ${tb} is not a standard design timebox (20 triage / 45 standard)`);
  }
  const ma = asI64(get(payload, "max_attempts") ?? null);
  if (ma !== null) {
    const kind = typeof get(payload, "kind") === "string" ? (get(payload, "kind") as string) : "";
    if (ma !== TASK_DEFAULT_MAX_ATTEMPTS) {
      warnings.push(
        `payload.max_attempts: design default for ${kind} is ${TASK_DEFAULT_MAX_ATTEMPTS}, packet says ${ma}`,
      );
    }
  }
  errors.push(...dedupeErrors(packet, payload));
  return [errors, warnings];
}

/** The #7030 band a payload's labels select, if any. */
export function labelBandPriority(payload: Json): number | null {
  if (!isObj(payload)) return null;
  const labels = get(payload, "labels");
  if (!Array.isArray(labels)) return null;
  const has = (name: string) => labels.some((l) => l === name);
  if (has("expedite")) return EXPEDITE_PRIORITY;
  if (has("next-up")) return NEXT_UP_PRIORITY;
  return null;
}

function cloneJson<T extends Json>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

/** task_from_packet: the task-row shape the enqueue verb inserts. */
export function taskFromPacket(packet: JsonObject, queue: string): JsonObject {
  let payload = cloneJson((get(packet, "payload") as Json) ?? {});
  if (!isObj(payload)) payload = {};
  const packetMax = get(packet, "max_attempts");
  const payloadMax = get(payload, "max_attempts");
  const maxAttempts: Json =
    packetMax !== undefined && packetMax !== null
      ? packetMax
      : payloadMax !== undefined
        ? payloadMax
        : TASK_DEFAULT_MAX_ATTEMPTS;
  const priorMirror = get(payload, "max_attempts");
  payload["max_attempts"] = maxAttempts;
  const size = new TextEncoder().encode(JSON.stringify(payload)).length;
  if (size > PAYLOAD_MAX_BYTES) {
    if (priorMirror === undefined) {
      delete payload["max_attempts"];
    } else {
      payload["max_attempts"] = priorMirror;
    }
  }
  const packetRbm = get(packet, "required_budget_minutes");
  const payloadRbm = get(payload, "required_budget_minutes");
  const requiredBudget: Json =
    packetRbm !== undefined && packetRbm !== null
      ? packetRbm
      : payloadRbm !== undefined && payloadRbm !== null
        ? payloadRbm
        : null;
  if (requiredBudget !== null) {
    payload["required_budget_minutes"] = requiredBudget;
  }
  const band = labelBandPriority(payload);
  const priority: Json = band !== null ? band : ((get(packet, "priority") as Json) ?? null);
  return {
    queue,
    manual_only: get(packet, "manual_only") === true,
    title: (get(packet, "title") as Json) ?? null,
    body: (get(packet, "body") as Json) ?? null,
    payload,
    dedupe_key: (get(packet, "dedupe_key") as Json) ?? null,
    priority,
    not_before: (get(packet, "not_before") as Json) ?? null,
    max_attempts: maxAttempts,
    required_budget_minutes: requiredBudget,
  };
}
