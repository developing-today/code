// Resolution-order parity with #9075 (#12584). Oracle: the native
// resolver, pkgs/fleet/crates/fleet/src/session_state.rs — its
// resolve() semantics and its own unit tests, transcribed case for
// case below (each case names the native test it transcribes). The
// order itself is recorded in three concordant places: #9075's
// acceptance ("explicit argument > env > pwd-file"), the design note
// doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy §14
// ("Resolution order: explicit argument > environment > pwd-file"),
// and the component-C design §3.2 (the same order, "with the
// pwd-file replaced per §4.2" by the CLI's per-workspace state
// file). The hosted env level keeps the native two variables, in
// native order: FLEET_SESSION_ID, then OPENCODE_SESSION_ID
// (pkgs/fleet/README, "Session identity in worktrees (#9075)").
//
// Same inputs, same resolution result, in the same order — including
// the failure modes: a malformed nonempty value at any consulted
// level fails closed instead of falling through, and a stored id
// that is not live is a named stale error, never a silent fallback.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  FLEET_SESSION_ENV,
  InMemorySessionStateStore,
  OPENCODE_SESSION_ENV,
  SessionResolutionError,
  resolveAndBindSession,
  resolveSessionId,
  type SessionResolution,
  type SessionStateStore,
} from "../src/session-resolution.ts";

interface ParityCase {
  /** The session_state.rs test this case transcribes. */
  native: string;
  label: string;
  explicit?: string | null;
  fleetEnv?: string | null;
  opencodeEnv?: string | null;
  stored?: string | null;
  /** Ids the liveness predicate confirms (native marker_is_live). */
  live?: string[];
  expect:
    | { session: string; source: "explicit" | "env" | "stored" }
    | { none: true }
    | { error: "invalid_session_id" | "stale_session_state" };
}

const CASES: ParityCase[] = [
  {
    native: "explicit_argument_wins_over_both_environments_and_pwd_file",
    label: "explicit beats both env vars and the stored state",
    explicit: "ses_explicit",
    fleetEnv: "ses_fleet",
    opencodeEnv: "ses_open",
    stored: "ses_file",
    live: [],
    expect: { session: "ses_explicit", source: "explicit" },
  },
  {
    native: "fleet_env_precedes_opencode_env_and_pwd_file",
    label: "FLEET_SESSION_ID beats OPENCODE_SESSION_ID and the stored state",
    fleetEnv: "ses_fleet",
    opencodeEnv: "ses_open",
    stored: "ses_file",
    live: [],
    expect: { session: "ses_fleet", source: "env" },
  },
  {
    native: "opencode_env_precedes_pwd_file",
    label: "OPENCODE_SESSION_ID beats the stored state",
    opencodeEnv: "ses_open",
    stored: "ses_file",
    live: [],
    expect: { session: "ses_open", source: "env" },
  },
  {
    native: "marker_is_used_only_when_live_and_stale_marker_is_explicit",
    label: "a live stored id is used",
    stored: "ses_file",
    live: ["ses_file"],
    expect: { session: "ses_file", source: "stored" },
  },
  {
    native: "marker_is_used_only_when_live_and_stale_marker_is_explicit",
    label: "an ended stored id is a named stale error, not a fallback",
    stored: "ses_ended",
    live: [],
    expect: { error: "stale_session_state" },
  },
  {
    native: "whitespace_values_are_absent",
    label: "a whitespace env value counts as absent",
    fleetEnv: "  ",
    expect: { none: true },
  },
  {
    native: "whitespace_values_are_absent",
    label: "a whitespace stored value counts as absent (same nonempty rule)",
    stored: "   ",
    expect: { none: true },
  },
  {
    native: "invalid_environment_and_marker_ids_fail_closed",
    label: "a malformed env id fails closed",
    fleetEnv: "not-a-session",
    live: ["not-a-session"],
    expect: { error: "invalid_session_id" },
  },
  {
    native: "invalid_environment_and_marker_ids_fail_closed",
    label: "a malformed stored id fails closed (format precedes liveness)",
    stored: "ses_bad-id",
    live: ["ses_bad-id"],
    expect: { error: "invalid_session_id" },
  },
  {
    native: "resolve() environment step; pkgs/fleet/README (#9075)",
    label: "a malformed env id fails closed before a live stored id is consulted",
    fleetEnv: "bogus",
    stored: "ses_file",
    live: ["ses_file"],
    expect: { error: "invalid_session_id" },
  },
  {
    native: "resolve() explicit step (checked_identity); parse_bind validation",
    label: "a malformed explicit id fails closed",
    explicit: "not-a-session",
    fleetEnv: "ses_fleet",
    expect: { error: "invalid_session_id" },
  },
  {
    native: "nonempty() applied to the explicit argument",
    label: "a whitespace explicit value falls through to the env level",
    explicit: "   ",
    fleetEnv: "ses_fleet",
    expect: { session: "ses_fleet", source: "env" },
  },
  {
    native: "resolve() fallthrough to Resolution::None",
    label: "no source anywhere resolves to none",
    expect: { none: true },
  },
];

function envOf(c: ParityCase): (name: string) => string | null {
  return (name) =>
    name === FLEET_SESSION_ENV ? (c.fleetEnv ?? null) : name === OPENCODE_SESSION_ENV ? (c.opencodeEnv ?? null) : null;
}

for (const c of CASES) {
  test(`parity [${c.native}] ${c.label}`, () => {
    const run = (): SessionResolution =>
      resolveSessionId({ explicit: c.explicit ?? null, env: envOf(c), stored: c.stored ?? null }, (id) =>
        (c.live ?? []).includes(id),
      );
    if ("session" in c.expect) {
      assert.deepEqual(run(), {
        kind: "session",
        sessionId: c.expect.session,
        source: c.expect.source,
      });
    } else if ("none" in c.expect) {
      assert.deepEqual(run(), { kind: "none" });
    } else {
      assert.throws(
        run,
        (e: unknown) =>
          e instanceof SessionResolutionError &&
          e.code === c.expect.error &&
          // The stale error names the offending id, as native's does.
          (c.expect.error !== "stale_session_state" || e.message.includes(c.stored ?? "")),
      );
    }
  });
}

test("parity [supplied_identity_is_persisted_for_a_later_bare_invocation] supplied identities persist; bare invocations recover them", () => {
  const store = new InMemorySessionStateStore();
  const none = (): string | null => null;
  const envWith = (fleet: string | null, opencode: string | null) => (name: string) =>
    name === FLEET_SESSION_ENV ? fleet : name === OPENCODE_SESSION_ENV ? opencode : null;

  // An env-supplied identity resolves without any liveness check and
  // is persisted to the stored state.
  assert.deepEqual(resolveAndBindSession({ env: envWith(null, "ses_active"), store, isLive: () => false }), {
    kind: "session",
    sessionId: "ses_active",
    source: "env",
  });
  assert.equal(store.read(), "ses_active");

  // A later bare invocation recovers it from the stored state.
  assert.deepEqual(resolveAndBindSession({ env: none, store, isLive: (id) => id === "ses_active" }), {
    kind: "session",
    sessionId: "ses_active",
    source: "stored",
  });

  // An explicit identity replaces the stored one.
  assert.deepEqual(resolveAndBindSession({ explicit: "ses_explicit", env: none, store, isLive: () => false }), {
    kind: "session",
    sessionId: "ses_explicit",
    source: "explicit",
  });
  assert.equal(store.read(), "ses_explicit");
  assert.deepEqual(resolveAndBindSession({ env: none, store, isLive: (id) => id === "ses_explicit" }), {
    kind: "session",
    sessionId: "ses_explicit",
    source: "stored",
  });
});

test("parity [explicit_and_environment_values_do_not_read_an_invalid_marker] supplied identities never read the stored state", () => {
  let reads = 0;
  const store: SessionStateStore = {
    read: () => {
      reads++;
      return "ses_bad-id";
    },
    write: () => {},
  };
  const none = (): string | null => null;
  assert.deepEqual(resolveAndBindSession({ explicit: "ses_explicit", env: none, store, isLive: () => true }), {
    kind: "session",
    sessionId: "ses_explicit",
    source: "explicit",
  });
  assert.deepEqual(
    resolveAndBindSession({
      env: (name) => (name === FLEET_SESSION_ENV ? "ses_env" : null),
      store,
      isLive: () => true,
    }),
    { kind: "session", sessionId: "ses_env", source: "env" },
  );
  assert.equal(reads, 0);
});
