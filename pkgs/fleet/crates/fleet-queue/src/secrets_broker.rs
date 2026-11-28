//! secrets-broker — resident Bitwarden Secrets Manager cache (code#5362).
//!
//! Fetches the vault's secrets ONCE per TTL via the `bws` CLI on the
//! laptop and serves them to token-authed local consumers, replacing the
//! per-secret `bws secret get` refresh that made hundreds of
//! repo-queue.py invocation pay 10-40 s. Binds the laptop tailnet IP
//! only; consumers reach it through the sandbox session proxy.
//!
//! Endpoints (header `X-Agent-Token` required except /health):
//!   GET /health        -> {ok, serving, secrets, last_refresh_unix, refreshing,
//!                          refresh_progress, oldest_secret_age_seconds}
//!                          | 503 while `serving` is false (no successful
//!                          refresh yet, #10868). While a refresh is in
//!                          flight, refresh_progress is {started_unix,
//!                          fetched, total}; it is null when idle (#13081).
//!   GET /secrets       -> {names: [...]}
//!   GET /secret/<name> -> {key, value, fetched_at_unix, age_seconds,
//!                          source} | 404 | 503 (not loaded yet)
//!
//! Every cached value carries the time the broker last fetched it
//! from Bitwarden (#6767), and /secret/<name> exposes that age plus
//! the value's source (cache vs a just-now read-through), so a
//! rotation runbook can tell whether it is verifying the broker or
//! Bitwarden itself. A refresh validates the complete one-call list
//! before atomically replacing the cache; any failed or partial list
//! preserves the previous values and watermark (#5504). A rotation
//! therefore lands within one TTL of the edit, bounded and observable
//! via the exposed age.
//!
//! A miss on a loaded cache is not final: the broker reads through
//! to Bitwarden for that one name (single-flighted per name) and
//! serves what it finds, so a secret minted moments ago is visible
//! immediately instead of at the next TTL refresh (#6275).
//!
//! The heal contract (#13081): a supervisor that restarts this
//! broker on a health verdict (the sandbox init-ensure prelude is
//! the live one) must not restart it while a refresh is in flight
//! and making progress — `refreshing` true with a present
//! `refresh_progress` whose `fetched` count advances between
//! probes, or a `secrets` count that reaches `refresh_progress.total`
//! when the one-call list lands. A first refresh over the whole
//! vault is a single `bws secret list` (seconds, #13948 S1), so the
//! patience window is sized to one list call plus install — not to
//! hundreds of per-secret fetches. Restart is for a broker that is
//! absent, not refreshing and not serving, or still not serving
//! after that window with no progress inside it.
//!
//! Env: SECRETS_BROKER_BIND (default 100.99.113.103:4800),
//!      SECRETS_BROKER_SOCKET (Unix socket dual-bind, #6216; default
//!      ~/.config/secrets-broker/broker.sock via fleet-config),
//!      SECRETS_BROKER_TOKEN_FILE (default ~/.config/secrets-broker/token),
//!      SECRETS_BROKER_WATERMARK_FILE (default
//!      ~/.config/secrets-broker/watermark, #13948 S2),
//!      SECRETS_BROKER_TTL_S (default 900, minimum 30 — validated
//!      at startup, #5860).
//!
//! S2 delta refresh (#13948 §5.1): each refresh is still the single
//! `bws secret list` call S1 landed, but entries are applied by
//! `revisionDate` against a durable watermark — the previous
//! successful refresh's start time minus WATERMARK_SKEW_S overlap,
//! persisted atomically to the watermark file and reloaded at
//! startup. Entries newer than the watermark are installed, the rest
//! are untouched, ids absent from the list are deleted. The
//! persisted watermark is the SINGLE quantity the peer chain
//! consumes (S3 exposes it, S4 compares and adopts it); no second
//! un-skewed timestamp exists.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tiny_http::{Header, Response, Server, StatusCode};

/// One cached secret: its value plus when the broker last fetched
/// it from Bitwarden (#6767). The age is what lets a consumer (or a
/// rotation runbook) distinguish "the broker is serving a value N
/// seconds old" from "Bitwarden itself holds this value". The `id`
/// is the Bitwarden secret id from the last list that carried this
/// name; it is what deletion-by-id-diff compares (#13948 S2). A
/// read-through install (#6275) does not know the id, so it stores
/// an empty id, which the delta apply retains conservatively — an
/// unknown id can never prove absence.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CachedSecret {
    id: String,
    value: String,
    fetched_at_unix: u64,
}

/// One in-flight refresh's progress (#13081): the unix time it
/// started, the fetches completed so far (either outcome — a
/// completed fetch is forward motion through the list), and the
/// total the authoritative list carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RefreshProgress {
    started_unix: u64,
    fetched: usize,
    total: usize,
}

struct State {
    secrets: RwLock<HashMap<String, CachedSecret>>,
    last_refresh: RwLock<Option<u64>>,
    /// The most recent vault-refresh failure, cleared by the next
    /// successful refresh. Served on the not-loaded 503 so a client
    /// can see WHY the broker has no secrets (#10743) instead of
    /// retry-looping a bare "not loaded yet". Status-only text by
    /// construction (BwsRunner discipline): never vault material.
    last_refresh_error: RwLock<Option<String>>,
    refreshing: RwLock<bool>,
    /// The in-flight refresh's progress (#13081): when it started,
    /// how many of the vault's fetches it has completed, and how
    /// many the authoritative list carries. Present only while a
    /// refresh is in flight; /health publishes it so a supervising
    /// heal can tell a slow-but-advancing refresh from a wedged
    /// one before it reaches for a restart.
    refresh_progress: RwLock<Option<RefreshProgress>>,
    /// In-flight read-through fetches, one gate per name (#6275):
    /// the first misser fetches from Bitwarden; concurrent missers
    /// for the same name wait on the gate instead of stampeding the
    /// vault with duplicate list+get pairs.
    inflight: Mutex<HashMap<String, Arc<FetchGate>>>,
}

/// The outcome slot of one in-flight read-through fetch: the leader
/// stores its result and notifies; waiters clone the result out.
struct FetchGate {
    result: Mutex<Option<Result<Option<String>, String>>>,
    done: Condvar,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn bws_token() -> Option<String> {
    if let Ok(t) = std::env::var("BWS_ACCESS_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    let home = crate::home::from_env().ok()?;
    for rel in [".config/bitwarden/access-token", ".config/bws/access-token"] {
        if let Ok(t) = std::fs::read_to_string(format!("{home}/{rel}")) {
            if !t.trim().is_empty() {
                return Some(t.trim().to_string());
            }
        }
    }
    None
}

/// Deadline for one `bws` child (#10871). A child that never returns
/// must fail its fetch, or its worker never finishes, the refresh's
/// join never returns, and the `refreshing` guard stays set forever.
const BWS_CHILD_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `bws` for the vault refresh, through the platform crate's
/// CommandRunner discipline (fleet-secrets, design §1): stdout on
/// success; on failure the error carries the exit status only —
/// never stderr, which could quote vault material. (The standalone
/// broker included a trimmed stderr excerpt in this log-only error;
/// the port deliberately adopts the stricter platform rule.)
struct BwsRunner {
    token: String,
    timeout: Duration,
}

impl fleet_secrets::CommandRunner for BwsRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        // A bare `bws` spawn dies with NotFound when the broker was
        // launched under a stripped PATH (#10743); resolve through
        // the well-known install locations instead.
        let resolved;
        let program: &std::path::Path = if program == "bws" {
            resolved = fleet_secrets::resolve_bws_program();
            &resolved
        } else {
            std::path::Path::new(program)
        };
        let mut child = Command::new(program)
            .args(args)
            .env("BWS_ACCESS_TOKEN", &self.token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn failed: {}", e.kind()))?;
        let mut pipe = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(out) = pipe.as_mut() {
                let _ = std::io::Read::read_to_end(out, &mut buf);
            }
            buf
        });
        let deadline = std::time::Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timed out after {}s", self.timeout.as_secs()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(format!("wait failed: {}", e.kind())),
            }
        };
        let stdout = reader.join().unwrap_or_default();
        if !status.success() {
            return Err(status.to_string());
        }
        String::from_utf8(stdout).map_err(|_| "output not utf8".to_string())
    }
}

fn bws_run(args: &[&str], token: &str) -> Result<String, String> {
    let runner = BwsRunner {
        token: token.to_string(),
        timeout: BWS_CHILD_TIMEOUT,
    };
    fleet_secrets::CommandRunner::run(&runner, "bws", args)
        .map_err(|e| format!("bws {} failed: {e}", args.join(" ")))
}

/// One full vault refresh: one `bws secret list` call carrying values,
/// validated strictly, then installed atomically (#13948 S1).
fn refresh(state: &Arc<State>) -> Result<usize, String> {
    {
        let mut r = state.refreshing.write().unwrap();
        if *r {
            return Err("refresh already in progress".into());
        }
        *r = true;
    }
    let result = do_refresh(state);
    // The refresh-error bookkeeping lives here, in the one shared
    // operation, so every caller records it — the background loop
    // AND the manual POST /refresh path, whose caller discards the
    // result (CodeRabbit review, #10757). The guard rejection
    // above returns before this point and never overwrites the
    // recorded error: a rejected overlap is not an attempt.
    match &result {
        Ok(_) => *state.last_refresh_error.write().unwrap() = None,
        Err(e) => *state.last_refresh_error.write().unwrap() = Some(e.clone()),
    }
    end_refresh_progress(state);
    *state.refreshing.write().unwrap() = false;
    result
}

/// Open the in-flight refresh's progress record (#13081): called
/// once the authoritative list has landed, so `total` is the real
/// vault size a heal sizes its patience against.
fn begin_refresh_progress(state: &Arc<State>, total: usize) {
    *state.refresh_progress.write().unwrap() = Some(RefreshProgress {
        started_unix: now_unix(),
        fetched: 0,
        total,
    });
}

/// Count one completed fetch against the in-flight refresh
/// (#13081). Either outcome counts: the fetch finished, so the
/// refresh is one step further through the list. A wedged refresh
/// is precisely one whose count stops moving.
fn note_refresh_fetch(state: &Arc<State>) {
    if let Some(p) = state.refresh_progress.write().unwrap().as_mut() {
        p.fetched += 1;
    }
}

/// Close the in-flight refresh's progress record (#13081): the
/// record exists only while a refresh is in flight, so a heal
/// never mistakes a finished refresh's final count for live
/// progress.
fn end_refresh_progress(state: &Arc<State>) {
    *state.refresh_progress.write().unwrap() = None;
}

/// Parse the `bws secret list` entries strictly: every entry must
/// carry a string `id`, `key`, and `value`. The list response is the
/// complete refresh payload, so any malformed entry fails the whole
/// parse before the cache is touched (#5504).
fn parse_list_entries(list: &Value) -> Result<Vec<(String, String, String)>, String> {
    let arr = list.as_array().ok_or("bws list is not an array")?;
    let mut items = Vec::with_capacity(arr.len());
    for (i, s) in arr.iter().enumerate() {
        let id = s
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("bws list entry {i} has a missing or non-string id"))?;
        let key = s
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("bws list entry {i} has a missing or non-string key"))?;
        let value = s
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("bws list entry {i} has a missing or non-string value"))?;
        items.push((id.to_string(), key.to_string(), value.to_string()));
    }
    Ok(items)
}

/// Skew overlap subtracted from the refresh start to form the
/// durable watermark (#13948 §5.1, Drew's "from maybe just before
/// the last refresh started"). A change timestamped during the
/// refresh window is never skipped: the next refresh's filter is
/// `revisionDate > watermark`, and the watermark sits a full skew
/// behind the start of the refresh that produced it, so an entry
/// revised mid-refresh still reads as newer. 60 s covers a whole
/// one-call list refresh (seconds, S1) with wide margin without
/// re-applying meaningful history on every tick.
pub const WATERMARK_SKEW_S: u64 = 60;

/// Default watermark file: alongside the broker token, under the
/// same `~/.config/secrets-broker` directory. Overridable with
/// `SECRETS_BROKER_WATERMARK_FILE` (tests point it at a temp dir).
fn watermark_file_path() -> Option<String> {
    if let Ok(p) = std::env::var("SECRETS_BROKER_WATERMARK_FILE") {
        if !p.trim().is_empty() {
            return Some(p);
        }
    }
    let home = crate::home::from_env().ok()?;
    Some(format!("{home}/.config/secrets-broker/watermark"))
}

/// Load the durable watermark written by the previous process, if
/// any. A missing or unreadable file is a cold start (full apply),
/// never a failure: re-applying the whole list is always safe, it
/// just costs one full install.
fn load_watermark() -> Option<u64> {
    let path = watermark_file_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    raw.trim().parse::<u64>().ok()
}

/// Persist the watermark atomically (write-temp-then-rename), so a
/// broker killed mid-refresh restarts into the pre-refresh watermark
/// and pre-refresh set, never a torn file (#13948 S2, #5504). The
/// file carries a bare decimal unix time plus newline; it names no
/// secret and holds no vault material. Best-effort at the call site:
/// a persist failure is logged and the in-memory watermark still
/// advances — the next process start simply falls back to a full
/// apply, which is safe.
fn persist_watermark(watermark: u64) -> Result<(), String> {
    let path = watermark_file_path().ok_or("no watermark file path (HOME unset)")?;
    let tmp = format!("{path}.tmp-{}", std::process::id());
    std::fs::write(&tmp, format!("{watermark}\n"))
        .map_err(|e| format!("watermark write failed: {}", e.kind()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("watermark rename failed: {}", e.kind()))?;
    Ok(())
}

/// Parse an RFC 3339 timestamp (`bws secret list` `revisionDate`,
/// e.g. `2026-10-07T07:23:52.457179600Z`) to unix seconds, without a
/// date-time crate (none in the workspace). Accepts `Z` and numeric
/// `±HH:MM` offsets, with or without a fractional second part.
/// Returns `None` on any shape it does not recognise; callers treat
/// an unparseable stamp as "changed" (apply the entry), never as a
/// reason to skip it.
fn parse_rfc3339_unix(s: &str) -> Option<u64> {
    let s = s.trim();
    let (dt, offset_s): (&str, i64) = if let Some(z) = s.strip_suffix('Z').or_else(|| s.strip_suffix('z')) {
        (z, 0)
    } else if s.len() >= 6 {
        let (head, tail) = s.split_at(s.len() - 6);
        let (sign_b, hh, mm) = (tail.as_bytes()[0], &tail[1..3], &tail[4..6]);
        if (sign_b != b'+' && sign_b != b'-') || &tail[3..4] != ":" {
            return None;
        }
        let hh: i64 = hh.parse().ok()?;
        let mm: i64 = mm.parse().ok()?;
        let off = hh * 3600 + mm * 60;
        (head, if sign_b == b'+' { off } else { -off })
    } else {
        return None;
    };
    let (date, time) = dt.split_once('T').or_else(|| dt.split_once('t'))?;
    let d: Vec<&str> = date.split('-').collect();
    let (y, mo, d): (i64, i64, i64) = match d.as_slice() {
        [y, mo, d] => (y.parse().ok()?, mo.parse().ok()?, d.parse().ok()?),
        _ => return None,
    };
    let time = time.split('.').next()?;
    let t: Vec<&str> = time.split(':').collect();
    let (h, mi, se): (i64, i64, i64) = match t.as_slice() {
        [h, mi, se] => (h.parse().ok()?, mi.parse().ok()?, se.parse().ok()?),
        _ => return None,
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    // days_from_civil (Howard Hinnant): days since 1970-01-01.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se - offset_s;
    u64::try_from(secs).ok()
}

/// One `bws secret list` entry's revision stamp, if present and
/// parseable. A missing or unparseable `revisionDate` is NOT a
/// validation failure — the entry is applied conservatively (it may
/// have changed; skipping it would risk serving stale). Only
/// id/key/value strictness can fail a refresh (#5504).
fn entry_revision_unix(entry: &Value) -> Option<u64> {
    entry
        .get("revisionDate")
        .and_then(|v| v.as_str())
        .and_then(parse_rfc3339_unix)
}

/// A fully validated list entry plus its revision stamp (None =
/// missing/unparseable, apply conservatively).
type DeltaEntry = (String, String, String, Option<u64>);

/// Validate the whole list strictly (id/key/value, #5504) and attach
/// each entry's revision stamp. Any malformed entry fails the whole
/// parse before the cache is touched.
fn parse_delta_entries(list: &Value) -> Result<Vec<DeltaEntry>, String> {
    let items = parse_list_entries(list)?;
    let arr = list.as_array().ok_or("bws list is not an array")?;
    Ok(items
        .into_iter()
        .zip(arr.iter())
        .map(|((id, key, value), raw)| (id, key, value, entry_revision_unix(raw)))
        .collect())
}

/// The outcome of one delta apply: how many listed entries were
/// installed, how many cached names were evicted by the id-set diff,
/// and the total the cache holds now. Logged by the refresh loop as
/// names/counts only — never values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeltaOutcome {
    applied: usize,
    deleted: usize,
    total: usize,
}

/// Apply one validated list as a delta against `watermark`
/// (#13948 §5.1): entries with `revisionDate` after the watermark —
/// or with no parseable stamp — are installed; every other entry is
/// untouched (its cached value AND its fetch time survive); ids
/// absent from the list are deleted locally. `watermark == None` is
/// the cold-start full load: every entry is installed.
///
/// Atomicity: the new map is built entirely from the read lock's
/// snapshot plus the validated entries, then swapped in under one
/// write lock together with the watermark advance. A failed or
/// partial list never reaches this function, so it can never evict
/// (#5504); a broker killed before the swap restarts into the
/// pre-refresh set and watermark.
fn apply_delta_refresh(
    state: &Arc<State>,
    entries: &[DeltaEntry],
    watermark: Option<u64>,
    fetched_at_unix: u64,
    watermark_new: u64,
) -> DeltaOutcome {
    let listed_keys: std::collections::HashSet<&str> =
        entries.iter().map(|(_, key, _, _)| key.as_str()).collect();
    let old = state.secrets.read().unwrap();
    let mut next: HashMap<String, CachedSecret> = HashMap::with_capacity(entries.len());
    let mut applied = 0usize;
    for (id, key, value, rev) in entries {
        let changed = match (watermark, rev) {
            (None, _) => true,
            (_, None) => true,
            (Some(wm), Some(r)) => *r > wm,
        };
        if changed {
            applied += 1;
            next.insert(
                key.clone(),
                CachedSecret {
                    id: id.clone(),
                    value: value.clone(),
                    fetched_at_unix,
                },
            );
        } else if let Some(kept) = old.get(key) {
            next.insert(key.clone(), kept.clone());
        } else {
            // Below-or-at the watermark but not previously cached
            // (e.g. evicted read-through, or a first refresh racing
            // a concurrent read-through): install it — the listing
            // is authoritative for the value it carries.
            applied += 1;
            next.insert(
                key.clone(),
                CachedSecret {
                    id: id.clone(),
                    value: value.clone(),
                    fetched_at_unix,
                },
            );
        }
    }
    // Deletion by id-set diff: a cached NAME the authoritative list
    // no longer carries is dropped when its Bitwarden id is known
    // (hence provably absent — the id set is the existence proof) and
    // retained when its id is unknown (read-through installs with no
    // recorded id: absence cannot be proven for them, and the next
    // refresh carrying their name normalises them).
    for (key, kept) in old.iter() {
        if listed_keys.contains(key.as_str()) {
            continue;
        }
        if kept.id.is_empty() {
            next.insert(key.clone(), kept.clone());
        }
    }
    // Evicted names: cached before, gone now.
    let deleted = old.keys().filter(|k| !next.contains_key(*k)).count();
    let total = next.len();
    drop(old);
    *state.secrets.write().unwrap() = next;
    // The persisted watermark is the SINGLE quantity the peer chain
    // consumes (S3 exposes it as `last_upstream_refresh_at`, S4
    // compares and adopts it): refresh-start minus the skew overlap.
    // No second un-skewed timestamp is kept for peer comparison.
    *state.last_refresh.write().unwrap() = Some(watermark_new);
    DeltaOutcome {
        applied,
        deleted,
        total,
    }
}
/// Resolve one key to its id in a `bws secret list` payload.
/// Ok(None) is a definite "not in the vault"; a key-matching entry
/// whose id is missing or non-string is a source failure (Err), not
/// an absence (#6568) — reporting it as not-in-vault would send
/// consumers a 404 for a secret that exists.
fn resolve_list_id(list: &Value, name: &str) -> Result<Option<String>, String> {
    let arr = list.as_array().ok_or("bws list is not an array")?;
    for s in arr {
        if s.get("key").and_then(|k| k.as_str()) == Some(name) {
            return match s.get("id").and_then(|i| i.as_str()) {
                Some(id) => Ok(Some(id.to_string())),
                None => Err(format!(
                    "bws list entry for key {name:?} has a missing or non-string id"
                )),
            };
        }
    }
    Ok(None)
}

fn do_refresh(state: &Arc<State>) -> Result<usize, String> {
    // The watermark is the START of the last successful refresh minus
    // the skew overlap (#13948 §5.1): record the start before the one
    // `bws secret list` call lands, so a change timestamped during
    // the refresh window still reads as newer on the next tick.
    let started_unix = now_unix();
    let watermark = *state.last_refresh.read().unwrap();
    let token = bws_token().ok_or("no Bitwarden access token available")?;
    let list_raw = bws_run(&["secret", "list", "-o", "json"], &token)?;
    let list: Value =
        serde_json::from_str(&list_raw).map_err(|e| format!("bws list not json: {e}"))?;
    let entries = parse_delta_entries(&list)?;
    let total = entries.len();
    // The complete list (including values) has landed and validated.
    // No cache mutation happens until validation is complete, so a
    // partial/failed list leaves the previous cache and watermark
    // untouched (#5504).
    begin_refresh_progress(state, total);
    for _ in 0..total {
        note_refresh_fetch(state);
    }
    let fetched_at_unix = now_unix();
    let watermark_new = started_unix.saturating_sub(WATERMARK_SKEW_S);
    let outcome = apply_delta_refresh(state, &entries, watermark, fetched_at_unix, watermark_new);
    // Durability is best-effort after a successful apply: the
    // in-memory watermark has already advanced, and a persist failure
    // merely degrades the next process start to a full apply (safe).
    if let Err(e) = persist_watermark(watermark_new) {
        crate::log::info("secrets-broker", &format!("watermark persist failed: {e}"));
    } else {
        crate::log::info(
            "secrets-broker",
            &format!(
                "delta refresh: applied {} of {}, deleted {}, total {}",
                outcome.applied, total, outcome.deleted, outcome.total
            ),
        );
    }
    Ok(outcome.total)
}

/// Atomically replace the cache with one fully validated list result.
/// Cold-start/full-load install path and the S1 back-compat seam:
/// failed or partial list results never reach this function,
/// preserving the prior cache and `last_refresh` (#5504). The
/// recorded `now` is the watermark the caller computed
/// (refresh-start minus skew); tests pass fixed times through.
fn install_list_refresh(state: &Arc<State>, items: &[(String, String, String)], now: u64) -> usize {
    let fetched_at_unix = now;
    let secrets = items
        .iter()
        .map(|(id, key, value)| {
            (
                key.clone(),
                CachedSecret {
                    id: id.clone(),
                    value: value.clone(),
                    fetched_at_unix,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let total = secrets.len();
    *state.secrets.write().unwrap() = secrets;
    *state.last_refresh.write().unwrap() = Some(now);
    total
}

/// Validate a complete list before applying it. Keeping parsing and
/// install behind this seam lets the regression test prove malformed
/// list payloads are non-mutating, not merely rejected by the parser.
#[cfg(test)]
fn apply_list_refresh(state: &Arc<State>, list: &Value, now: u64) -> Result<usize, String> {
    let items = parse_list_entries(list)?;
    Ok(install_list_refresh(state, &items, now))
}

/// One name, straight from Bitwarden: resolve the key to its id via
/// the list, then get the value — the read-through path for a single
/// cache miss (#6275). Ok(None) is a definite "not in the vault"; Err
/// is a source failure (its detail carries the exit status only, per
/// BwsRunner's discipline).
fn fetch_one(name: &str) -> Result<Option<String>, String> {
    let token = bws_token().ok_or("no Bitwarden access token available")?;
    let list_raw = bws_run(&["secret", "list", "-o", "json"], &token)?;
    let list: Value =
        serde_json::from_str(&list_raw).map_err(|e| format!("bws list not json: {e}"))?;
    let id = resolve_list_id(&list, name)?;
    let Some(id) = id else {
        return Ok(None);
    };
    let got_raw = bws_run(&["secret", "get", &id, "-o", "json"], &token)?;
    let got: Value =
        serde_json::from_str(&got_raw).map_err(|e| format!("bws get not json: {e}"))?;
    match got.get("value").and_then(|v| v.as_str()) {
        Some(v) => Ok(Some(v.to_string())),
        None => Err("bws get response has no string value".to_string()),
    }
}

/// Read-through on a cache miss (#6275; Drew 2026-11-05: "broker
/// should go get the secret pass through"): fetch the one name from
/// Bitwarden and install it in the cache, so a just-minted secret
/// resolves immediately. Only the fetch's success is cached — a
/// not-found or a failure is returned but never installed, so the
/// next miss retries against the source.
fn fetch_through(state: &Arc<State>, name: &str) -> Result<Option<String>, String> {
    fetch_through_with(state, name, &fetch_one)
}

fn fetch_through_with(
    state: &Arc<State>,
    name: &str,
    fetch: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    // A refresh (or an earlier fetch) may have landed the value
    // since the caller missed.
    if let Some(c) = state.secrets.read().unwrap().get(name) {
        return Ok(Some(c.value.clone()));
    }
    // Join the in-flight fetch for this name, or become its leader.
    let (gate, leader) = {
        let mut inflight = state.inflight.lock().unwrap();
        match inflight.get(name) {
            Some(g) => (Arc::clone(g), false),
            None => {
                let g = Arc::new(FetchGate {
                    result: Mutex::new(None),
                    done: Condvar::new(),
                });
                inflight.insert(name.to_string(), Arc::clone(&g));
                (g, true)
            }
        }
    };
    if !leader {
        let mut result = gate.result.lock().unwrap();
        while result.is_none() {
            result = gate.done.wait(result).unwrap();
        }
        return result.clone().unwrap();
    }
    let result = fetch(name);
    match &result {
        Ok(Some(v)) => {
            state.secrets.write().unwrap().insert(
                name.to_string(),
                CachedSecret {
                    // Read-through knows the value but not the id
                    // (fetch_one resolves it internally and drops
                    // it): empty id, retained conservatively by the
                    // delta's id-set diff until a list carries it.
                    id: String::new(),
                    value: v.clone(),
                    fetched_at_unix: now_unix(),
                },
            );
            crate::log::info(
                "secrets-broker",
                &format!("read-through: fetched {name:?} from Bitwarden on a cache miss"),
            );
        }
        Ok(None) => crate::log::info(
            "secrets-broker",
            &format!("read-through: {name:?} is not in Bitwarden"),
        ),
        Err(e) => crate::log::info(
            "secrets-broker",
            &format!("read-through: fetch of {name:?} failed: {e}"),
        ),
    }
    *gate.result.lock().unwrap() = Some(result.clone());
    gate.done.notify_all();
    state.inflight.lock().unwrap().remove(name);
    result
}

fn json_response(status: u16, body: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let data = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    Response::from_data(data)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
}

fn token_matches(provided: Option<&str>, expected: &str) -> bool {
    // #6320 defence in depth: an empty expected token never authenticates,
    // not even against an empty X-Agent-Token header.
    if expected.is_empty() {
        return false;
    }
    provided.map(|v| v == expected).unwrap_or(false)
}

/// Pre-body gate for the broker transports (#6515, PR #6514 thread
/// 4492609604). Oracle: an unauthorized or oversized request is
/// refused from its headers alone, before a single body byte is
/// read. Returns the refusal to send, or `None` to proceed to the
/// body. The only unauthenticated route is the health probe,
/// mirroring `handle_request`.
fn pre_body_refusal(
    method: &str,
    target: &str,
    provided_token: Option<&str>,
    token: &str,
    content_length: Option<u64>,
) -> Option<(u16, Value)> {
    let path = target.split('?').next().unwrap_or("");
    if method == "GET" && path == "/health" {
        return None;
    }
    if !token_matches(provided_token, token) {
        return Some((401, json!({"error": "unauthorized"})));
    }
    if let Some(n) = content_length {
        if n > crate::uds_http::MAX_BODY_BYTES as u64 {
            return Some((413, json!({"error": "request body too large"})));
        }
    }
    None
}

/// Transport-agnostic request handling (#6216): the TCP loop adapts
/// tiny_http requests to this function and the UDS loop adapts
/// Unix-socket requests; routes and auth are identical on both
/// transports by construction. Returns (status, JSON body). The
/// request body is part of the shared shape but unused here: the one
/// body-carrying route, POST /refresh, ignores its body.
fn handle_request(
    state: &Arc<State>,
    token: &str,
    method: &str,
    url: &str,
    provided_token: Option<&str>,
) -> (u16, Value) {
    let path = url.split('?').next().unwrap_or("").to_string();
    if method == "GET" && path == "/health" {
        let now = now_unix();
        let oldest_secret_age_seconds = state
            .secrets
            .read()
            .unwrap()
            .values()
            .map(|c| now.saturating_sub(c.fetched_at_unix))
            .max();
        let last_refresh_unix = state.last_refresh.read().unwrap().clone();
        let serving = last_refresh_unix.is_some();
        // The in-flight refresh's progress (#13081): an object
        // while a refresh is running, null when idle, so a heal
        // reading this body can apply the refreshing-aware
        // patience rule instead of a bare not-serving verdict.
        let refresh_progress = state.refresh_progress.read().unwrap().map(|p| {
            json!({
                "started_unix": p.started_unix,
                "fetched": p.fetched,
                "total": p.total,
            })
        });
        (
            if serving { 200 } else { 503 },
            json!({
                "ok": serving,
                "serving": serving,
                "secrets": state.secrets.read().unwrap().len(),
                "last_refresh_unix": last_refresh_unix,
                "refreshing": *state.refreshing.read().unwrap(),
                "refresh_progress": refresh_progress,
                "oldest_secret_age_seconds": oldest_secret_age_seconds,
            }),
        )
    } else if !token_matches(provided_token, token) {
        (401, json!({"error": "unauthorized"}))
    } else if method == "GET" && path == "/secrets" {
        let mut names: Vec<String> = state.secrets.read().unwrap().keys().cloned().collect();
        names.sort();
        (200, json!({"names": names}))
    } else if method == "POST" && path == "/refresh" {
        let st = Arc::clone(state);
        std::thread::spawn(move || {
            let _ = refresh(&st);
        });
        (202, json!({"ok": true, "refresh": "started"}))
    } else if method == "GET" && path.starts_with("/secret/") {
        let name = path.trim_start_matches("/secret/").to_string();
        let name = url_decode(&name);
        let cached = state.secrets.read().unwrap().get(&name).cloned();
        match cached {
            Some(c) => (
                200,
                json!({
                    "key": name,
                    "value": c.value,
                    "fetched_at_unix": c.fetched_at_unix,
                    "age_seconds": now_unix().saturating_sub(c.fetched_at_unix),
                    "source": "cache",
                }),
            ),
            None => {
                if state.last_refresh.read().unwrap().is_none() {
                    let mut body = json!({"error": "secrets not loaded yet"});
                    if let Some(e) = state.last_refresh_error.read().unwrap().clone() {
                        body["refresh_error"] = json!(e);
                    }
                    (503, body)
                } else {
                    // A miss on a loaded cache reads through to
                    // Bitwarden for this one name (#6275) before the
                    // name is declared absent: a secret minted since
                    // the last refresh resolves now, not at the next
                    // TTL tick. A source failure is a loud 502, never
                    // a silent 404-as-if-absent.
                    match fetch_through(state, &name) {
                        Ok(Some(v)) => (
                            200,
                            json!({
                                "key": name,
                                "value": v,
                                "fetched_at_unix": now_unix(),
                                "age_seconds": 0,
                                "source": "bitwarden-read-through",
                            }),
                        ),
                        Ok(None) => (404, json!({"error": "no such secret"})),
                        Err(_) => (502, json!({"error": "bitwarden read-through failed"})),
                    }
                }
            }
        }
    } else {
        (404, json!({"error": "not found"}))
    }
}

/// Default vault-refresh interval when SECRETS_BROKER_TTL_S is unset.
pub const DEFAULT_TTL_S: u64 = 900;

/// Smallest accepted SECRETS_BROKER_TTL_S. One interval costs a full
/// vault refresh (one `bws secret list`, #13948 S1); below this floor
/// the loop spends its life inside bws — at 0 it never sleeps at all
/// (#5860 measured ~110 full refreshes/sec).
/// Startup refuses smaller values rather than clamping: a broker
/// silently running at a different rate than configured is its own
/// surprise on the credential path.
pub const MIN_TTL_S: u64 = 30;

/// The outcome of parsing SECRETS_BROKER_TTL_S: the interval to run
/// with, plus a warning the caller must surface when the operator's
/// value could not be honored as written.
#[derive(Debug)]
pub struct TtlParse {
    pub ttl_s: u64,
    pub warning: Option<String>,
}

/// Parse and validate SECRETS_BROKER_TTL_S (#5860). Pure, so the policy
/// is pinned by unit tests:
/// - unset -> DEFAULT_TTL_S, no warning (the documented default);
/// - unparseable -> DEFAULT_TTL_S with a warning naming the value —
///   never the old silent fallback;
/// - parsed but below MIN_TTL_S (including 0) -> Err naming the
///   variable, the value, and the minimum; startup refuses to run.
pub fn parse_ttl_s(raw: Option<&str>) -> Result<TtlParse, String> {
    let Some(raw) = raw else {
        return Ok(TtlParse {
            ttl_s: DEFAULT_TTL_S,
            warning: None,
        });
    };
    match raw.trim().parse::<u64>() {
        Ok(v) if v < MIN_TTL_S => Err(format!(
            "SECRETS_BROKER_TTL_S={raw:?} is below the minimum of {MIN_TTL_S}s: one interval is a full vault refresh, and 0 spins that loop with no sleep at all (#5860) — refusing to start"
        )),
        Ok(v) => Ok(TtlParse {
            ttl_s: v,
            warning: None,
        }),
        Err(_) => Ok(TtlParse {
            ttl_s: DEFAULT_TTL_S,
            warning: Some(format!(
                "SECRETS_BROKER_TTL_S={raw:?} is not a valid number of seconds; using the default of {DEFAULT_TTL_S}s"
            )),
        }),
    }
}

pub fn run() -> i32 {
    use std::io::Read as _;
    // Stage-2 broker resilience: on the sandbox the broker is a
    // local capability, never a LAN surface — force loopback
    // regardless of the environment, so a stale or widened
    // SECRETS_BROKER_BIND can never expose the vault off-host.
    // The spawn in `fleet init` sets the loopback bind explicitly;
    // this is the second enforcement, at the bind itself.
    let sandbox = std::env::var("FLEET_LOCATION")
        .ok()
        .is_some_and(|v| v.trim() == "sandbox");
    let bind = if sandbox {
        "127.0.0.1:4800".to_string()
    } else {
        std::env::var("SECRETS_BROKER_BIND").unwrap_or_else(|_| "100.99.113.103:4800".to_string())
    };
    let home = match crate::home::from_env() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("secrets-broker: {error}");
            return 2;
        }
    };
    let token_file = std::env::var("SECRETS_BROKER_TOKEN_FILE")
        .unwrap_or_else(|_| format!("{home}/.config/secrets-broker/token"));
    let token = match crate::token_file::load_token_file(&token_file) {
        Ok(v) => v,
        Err(e) => {
            crate::log::info("secrets-broker", &e);
            return 3;
        }
    };
    let ttl_s = match parse_ttl_s(std::env::var("SECRETS_BROKER_TTL_S").ok().as_deref()) {
        Ok(parsed) => {
            if let Some(warning) = &parsed.warning {
                crate::log::info("secrets-broker", warning);
                eprintln!("secrets-broker: {warning}");
            }
            parsed.ttl_s
        }
        Err(error) => {
            eprintln!("secrets-broker: {error}");
            return 2;
        }
    };

    // The durable watermark (#13948 §5.1): the previous process's
    // refresh-start-minus-skew, reloaded here so a restart resumes
    // delta filtering where the last successful refresh left off. A
    // missing file is a cold start — the next refresh applies the
    // whole list. This single quantity is also what the peer chain
    // consumes (S3 `last_upstream_refresh_at`, S4 compare-and-adopt).
    let loaded_watermark = load_watermark();
    if loaded_watermark.is_some() {
        crate::log::info("secrets-broker", "loaded durable watermark from disk");
    }
    let state = Arc::new(State {
        secrets: RwLock::new(HashMap::new()),
        last_refresh: RwLock::new(loaded_watermark),
        last_refresh_error: RwLock::new(None),
        refreshing: RwLock::new(false),
        refresh_progress: RwLock::new(None),
        inflight: Mutex::new(HashMap::new()),
    });

    // Background refresh loop: initial load + TTL refresh,
    // stale-while-revalidate (the cache keeps serving during a refresh,
    // and a failed refresh keeps the old values).
    {
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            loop {
                match refresh(&state) {
                    Ok(n) => crate::log::info("secrets-broker", &format!("refreshed {n} secrets")),
                    Err(e) => crate::log::info("secrets-broker", &format!("refresh failed: {e}")),
                }
                std::thread::sleep(Duration::from_secs(ttl_s));
            }
        });
    }

    let server = match Server::http(&bind) {
        Ok(server) => server,
        Err(e) => {
            crate::log::info("secrets-broker", &format!("cannot bind {bind}: {e:?}"));
            return 3;
        }
    };
    crate::log::info("secrets-broker", &format!("listening on {bind}"));

    // Dual-bind (#6216): the same handler over a Unix socket. A bind
    // failure or a live owner degrades to TCP only, never to exit:
    // the TCP listener is the established surface.
    let socket_path = fleet_config::Config::load()
        .unwrap_or_default()
        .secrets_broker_socket();
    match crate::uds_http::bind_socket(&socket_path) {
        Ok(Some(listener)) => {
            let st = Arc::clone(&state);
            let tok = token.clone();
            crate::log::info(
                "secrets-broker",
                &format!("listening on uds {}", socket_path.display()),
            );
            std::thread::spawn(move || {
                let pre_tok = tok.clone();
                crate::uds_http::serve_with_precheck(
                    listener,
                    move |head| {
                        // #6515: same pre-body gate as the TCP loop.
                        let content_length = head
                            .header("Content-Length")
                            .and_then(|v| v.parse::<u64>().ok());
                        pre_body_refusal(
                            &head.method,
                            &head.target,
                            head.header("X-Agent-Token"),
                            &pre_tok,
                            content_length,
                        )
                        .map(|(status, value)| {
                            (
                                status,
                                serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
                            )
                        })
                    },
                    move |req| {
                        let (status, value) = handle_request(
                            &st,
                            &tok,
                            &req.method,
                            &req.target,
                            req.header("X-Agent-Token"),
                        );
                        (
                            status,
                            serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
                        )
                    },
                );
            });
        }
        Ok(None) => crate::log::info(
            "secrets-broker",
            &format!(
                "uds {} already owned by a live broker; TCP only",
                socket_path.display()
            ),
        ),
        Err(e) => crate::log::info(
            "secrets-broker",
            &format!("cannot bind uds {}: {e}; TCP only", socket_path.display()),
        ),
    }

    for mut req in server.incoming_requests() {
        let state = Arc::clone(&state);
        let token = token.clone();
        std::thread::spawn(move || {
            let url = req.url().to_string();
            let provided = req
                .headers()
                .iter()
                .find(|h| h.field.as_str().to_ascii_lowercase() == "x-agent-token")
                .map(|h| h.value.as_str().to_string());
            // #6515: authenticate and size-check from the headers
            // before draining any body byte (tiny_http 0.12 exposes
            // no per-connection read timeout; see queue_broker).
            let content_length = req
                .headers()
                .iter()
                .find(|h| h.field.as_str().to_ascii_lowercase() == "content-length")
                .and_then(|h| h.value.as_str().parse::<u64>().ok());
            if let Some((status, value)) = pre_body_refusal(
                req.method().as_str(),
                &url,
                provided.as_deref(),
                &token,
                content_length,
            ) {
                let _ = req.respond(json_response(status, value));
                return;
            }
            if req.method().as_str() == "POST" {
                // POST /refresh's body is ignored by the handler;
                // drain it (capped, #6515) as the pre-#6216 loop did.
                let cap = (crate::uds_http::MAX_BODY_BYTES + 1) as u64;
                let mut buf = Vec::new();
                let _ = req.as_reader().take(cap).read_to_end(&mut buf);
                if buf.len() > crate::uds_http::MAX_BODY_BYTES {
                    let _ = req.respond(json_response(
                        413,
                        json!({"error": "request body too large"}),
                    ));
                    return;
                }
            }
            let (status, value) = handle_request(
                &state,
                &token,
                req.method().as_str(),
                &url,
                provided.as_deref(),
            );
            let _ = req.respond(json_response(status, value));
        });
    }
    // incoming_requests() ends only when the server stops.
    0
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        // A path segment is not a form query: '+' is a literal plus
        // here (RFC 3986), never a space. Only %XX decodes (issue #5584).
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        CachedSecret, DEFAULT_TTL_S, DeltaOutcome, MIN_TTL_S, State, WATERMARK_SKEW_S,
        apply_delta_refresh, apply_list_refresh, begin_refresh_progress, end_refresh_progress,
        entry_revision_unix, fetch_through_with, handle_request, install_list_refresh,
        load_watermark, note_refresh_fetch, now_unix, parse_delta_entries, parse_list_entries,
        parse_rfc3339_unix, parse_ttl_s, persist_watermark, pre_body_refusal, resolve_list_id,
        url_decode,
    };
    use std::time::Duration;

    #[test]
    fn pre_body_refusal_decides_from_headers_alone() {
        // #6515 oracle (thread 4492609604): the gate's answer
        // depends only on method/target/token/declared length —
        // never on body bytes — so an unauthorized or oversized
        // request is refused before its body is read.
        let over = crate::uds_http::MAX_BODY_BYTES as u64 + 1;
        assert_eq!(
            pre_body_refusal("POST", "/refresh", None, "tok", Some(over)).map(|r| r.0),
            Some(401)
        );
        assert_eq!(
            pre_body_refusal("POST", "/refresh", Some("tok"), "tok", Some(over)).map(|r| r.0),
            Some(413)
        );
        assert_eq!(
            pre_body_refusal("POST", "/refresh", Some("tok"), "tok", Some(0)),
            None
        );
        // The health probe stays unauthenticated, as in handle_request.
        assert_eq!(pre_body_refusal("GET", "/health", None, "tok", None), None);
    }
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, RwLock};

    fn empty_state() -> Arc<State> {
        Arc::new(State {
            secrets: RwLock::new(HashMap::new()),
            last_refresh: RwLock::new(Some(1)),
            last_refresh_error: RwLock::new(None),
            refreshing: RwLock::new(false),
            refresh_progress: RwLock::new(None),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    #[test]
    fn health_reports_serving_only_after_a_successful_refresh() {
        // oracle (#10868): a live process whose refresh never
        // succeeded (stripped PATH, bws unspawnable) is not serving,
        // so the health probe must fail and init-ensure can replace
        // it; a loaded broker answers 200 with serving=true.
        let state = empty_state();
        *state.last_refresh.write().unwrap() = None;
        let (status, body) = handle_request(&state, "tok", "GET", "/health", None);
        assert_eq!(status, 503);
        assert_eq!(body["serving"], false);
        assert_eq!(body["ok"], false);
        assert!(body.get("refresh_error").is_none());

        *state.last_refresh.write().unwrap() = Some(1);
        let (status, body) = handle_request(&state, "tok", "GET", "/health", None);
        assert_eq!(status, 200);
        assert_eq!(body["serving"], true);
        assert_eq!(body["ok"], true);
    }

    #[test]
    fn health_publishes_refresh_progress_only_while_refreshing() {
        // oracle (#13081): a heal deciding whether to restart a
        // not-yet-serving broker must be able to see that a first
        // refresh is in flight and how far through the vault it
        // is. In flight, /health carries the progress record
        // (start, fetched, total); idle, the field is null — a
        // finished refresh's final count is never mistaken for
        // live progress.
        let state = empty_state();
        *state.last_refresh.write().unwrap() = None;
        let (_, body) = handle_request(&state, "tok", "GET", "/health", None);
        assert_eq!(body["refresh_progress"], serde_json::Value::Null);

        *state.refreshing.write().unwrap() = true;
        begin_refresh_progress(&state, 221);
        note_refresh_fetch(&state);
        note_refresh_fetch(&state);
        let (status, body) = handle_request(&state, "tok", "GET", "/health", None);
        assert_eq!(status, 503);
        assert_eq!(body["serving"], false);
        assert_eq!(body["refreshing"], true);
        assert_eq!(body["refresh_progress"]["total"], 221);
        assert_eq!(body["refresh_progress"]["fetched"], 2);
        assert!(
            body["refresh_progress"]["started_unix"].as_u64().is_some(),
            "progress carries its start: {body}"
        );

        end_refresh_progress(&state);
        let (_, body) = handle_request(&state, "tok", "GET", "/health", None);
        assert_eq!(body["refresh_progress"], serde_json::Value::Null);
    }

    #[test]
    fn atomic_install_serves_the_whole_set_at_once() {
        // Oracle (#13948 S1): the one-call refresh swaps the whole
        // cache at once — a loaded broker serves every listed name,
        // and `serving` flips with the completed install.
        let state = empty_state();
        *state.last_refresh.write().unwrap() = None;
        let n = install_list_refresh(
            &state,
            &[
                ("id-a".to_string(), "alpha".to_string(), "v-a".to_string()),
                ("id-b".to_string(), "beta".to_string(), "v-b".to_string()),
            ],
            7,
        );
        assert_eq!(n, 2);
        let (status, body) = handle_request(&state, "tok", "GET", "/secret/alpha", Some("tok"));
        assert_eq!(status, 200);
        assert_eq!(body["fetched_at_unix"], 7);
        let (status, _) = handle_request(&state, "tok", "GET", "/secret/beta", Some("tok"));
        assert_eq!(status, 200);
    }

    #[test]
    fn refresh_progress_counts_completions_and_stops_at_the_end() {
        // oracle (#13081): the fetched count is the heal's motion
        // signal — it advances once per completed fetch and the
        // record closes with the refresh, so a wedged refresh
        // (count frozen across probes) is distinguishable from a
        // slow one (count advancing).
        let state = empty_state();
        begin_refresh_progress(&state, 3);
        for want in 1..=3 {
            note_refresh_fetch(&state);
            let p = state.refresh_progress.read().unwrap().expect("in flight");
            assert_eq!(p.fetched, want);
            assert_eq!(p.total, 3);
        }
        end_refresh_progress(&state);
        assert!(state.refresh_progress.read().unwrap().is_none());
        // A stray completion outside a refresh records nothing.
        note_refresh_fetch(&state);
        assert!(state.refresh_progress.read().unwrap().is_none());
    }

    #[test]
    fn not_loaded_503_names_the_refresh_failure() {
        // oracle (#10743): a broker that has never loaded because
        // its refresh is failing says why on the 503, so a client
        // surfaces the cause instead of a bare "not loaded yet".
        let state = empty_state();
        *state.last_refresh.write().unwrap() = None;
        *state.last_refresh_error.write().unwrap() =
            Some("bws secret list -o json failed: spawn failed: entity not found".to_string());
        let (status, body) = handle_request(&state, "tok", "GET", "/secret/alpha", Some("tok"));
        assert_eq!(status, 503);
        assert_eq!(body["error"], "secrets not loaded yet");
        assert_eq!(
            body["refresh_error"],
            "bws secret list -o json failed: spawn failed: entity not found"
        );
    }

    #[test]
    fn read_through_installs_the_fetched_value_in_the_cache() {
        let state = empty_state();
        let calls = AtomicUsize::new(0);
        let fetch = |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some("fixture-value-not-a-secret".to_string()))
        };
        let v = fetch_through_with(&state, "new.name", &fetch).unwrap();
        assert_eq!(v.as_deref(), Some("fixture-value-not-a-secret"));
        assert_eq!(
            state
                .secrets
                .read()
                .unwrap()
                .get("new.name")
                .map(|c| c.value.as_str()),
            Some("fixture-value-not-a-secret")
        );
        // Second read is a cache hit: no further fetch.
        let v = fetch_through_with(&state, "new.name", &fetch).unwrap();
        assert_eq!(v.as_deref(), Some("fixture-value-not-a-secret"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn read_through_not_found_is_returned_but_never_cached() {
        let state = empty_state();
        let calls = AtomicUsize::new(0);
        let fetch = |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        };
        assert_eq!(fetch_through_with(&state, "absent.name", &fetch), Ok(None));
        assert!(!state.secrets.read().unwrap().contains_key("absent.name"));
        // No negative caching: the next miss asks the source again.
        assert_eq!(fetch_through_with(&state, "absent.name", &fetch), Ok(None));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn read_through_failure_is_returned_but_never_cached() {
        let state = empty_state();
        let fetch = |_: &str| Err("exit status 1".to_string());
        let err = fetch_through_with(&state, "any.name", &fetch).unwrap_err();
        assert_eq!(err, "exit status 1");
        assert!(!state.secrets.read().unwrap().contains_key("any.name"));
    }

    #[test]
    fn concurrent_misses_for_one_name_fetch_exactly_once() {
        let state = empty_state();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let state = Arc::clone(&state);
            let calls = Arc::clone(&calls);
            handles.push(std::thread::spawn(move || {
                let fetch = |_: &str| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    Ok(Some("fixture-value-not-a-secret".to_string()))
                };
                fetch_through_with(&state, "hot.name", &fetch)
            }));
        }
        for h in handles {
            let v = h.join().unwrap().unwrap();
            assert_eq!(v.as_deref(), Some("fixture-value-not-a-secret"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn url_decode_keeps_plus_literal_in_a_path_segment() {
        // A secret name containing + is client-legal (fleet-secrets
        // validate_name permits it) and reaches the broker raw; decoding
        // it to a space looked the name up under the wrong spelling.
        assert_eq!(url_decode("a+b"), "a+b");
        assert_eq!(url_decode("+"), "+");
    }

    #[test]
    fn url_decode_still_decodes_percent_escapes() {
        assert_eq!(url_decode("a%2Bb"), "a+b");
        assert_eq!(url_decode("a%20b"), "a b");
        assert_eq!(url_decode("alpha"), "alpha");
        // Malformed escapes stay literal.
        assert_eq!(url_decode("a%2"), "a%2");
        assert_eq!(url_decode("a%zz"), "a%zz");
    }

    #[test]
    fn parse_list_entries_keeps_well_formed_entries_in_order() {
        let list = serde_json::json!([
            {"id": "id-1", "key": "alpha", "value": "x"},
            {"id": "id-2", "key": "beta", "value": "y"}
        ]);
        assert_eq!(
            parse_list_entries(&list).unwrap(),
            vec![
                ("id-1".to_string(), "alpha".to_string(), "x".to_string()),
                ("id-2".to_string(), "beta".to_string(), "y".to_string())
            ]
        );
    }

    #[test]
    fn parse_list_entries_fails_whole_on_a_malformed_entry() {
        // #6568/#5504: any entry with a missing or non-string id,
        // key, or value fails the parse, so a refresh can never
        // install a partial listing and evict a cached secret.
        for bad in [
            serde_json::json!([{"key": "alpha"}]),
            serde_json::json!([{"id": 7, "key": "alpha", "value": "x"}]),
            serde_json::json!([{"id": "id-1", "value": "x"}]),
            serde_json::json!([{"id": "id-1", "key": 7, "value": "x"}]),
            serde_json::json!([{"id": "id-1", "key": "alpha"}]),
            serde_json::json!([{"id": "id-1", "key": "alpha", "value": 7}]),
            serde_json::json!([{"id": "id-1", "key": "alpha", "value": "x"}, {"id": "id-2", "key": "beta"}]),
        ] {
            assert!(parse_list_entries(&bad).is_err(), "accepted {bad}");
        }
        assert!(parse_list_entries(&serde_json::json!({"not": "array"})).is_err());
    }

    #[test]
    fn resolve_list_id_distinguishes_absent_from_malformed() {
        let list = serde_json::json!([
            {"id": "id-1", "key": "alpha"},
            {"id": "id-2", "key": "beta"}
        ]);
        assert_eq!(
            resolve_list_id(&list, "beta").unwrap(),
            Some("id-2".to_string())
        );
        assert_eq!(resolve_list_id(&list, "absent").unwrap(), None);
        assert_eq!(
            resolve_list_id(&serde_json::json!([]), "alpha").unwrap(),
            None
        );
        // #6568: a key-matching entry with a missing or non-string
        // id is a source failure, not "not in the vault".
        let missing = serde_json::json!([{"key": "alpha"}]);
        assert!(resolve_list_id(&missing, "alpha").is_err());
        let non_string = serde_json::json!([{"id": 7, "key": "alpha"}]);
        assert!(resolve_list_id(&non_string, "alpha").is_err());
        assert!(resolve_list_id(&serde_json::json!({"not": "array"}), "alpha").is_err());
    }

    fn seeded_state(entries: &[(&str, &str, u64)]) -> Arc<State> {
        let state = empty_state();
        {
            let mut secrets = state.secrets.write().unwrap();
            for (k, v, at) in entries {
                secrets.insert(
                    k.to_string(),
                    CachedSecret {
                        // Fixture predates id tracking: unknown id,
                        // retained conservatively by the delta diff.
                        id: String::new(),
                        value: v.to_string(),
                        fetched_at_unix: *at,
                    },
                );
            }
        }
        state
    }

    /// S2 fixture: cache entries with known Bitwarden ids, as a
    /// successful refresh would leave them.
    fn seeded_state_ids(entries: &[(&str, &str, &str, u64)]) -> Arc<State> {
        let state = empty_state();
        {
            let mut secrets = state.secrets.write().unwrap();
            for (id, k, v, at) in entries {
                secrets.insert(
                    k.to_string(),
                    CachedSecret {
                        id: id.to_string(),
                        value: v.to_string(),
                        fetched_at_unix: *at,
                    },
                );
            }
        }
        state
    }

    #[test]
    fn install_list_refresh_replaces_cache_and_drops_delisted() {
        // Oracle (#13948 S1): a validated complete list replaces the
        // whole cache; a cached name absent from the list is evicted
        // (the list is authoritative for existence, #6568) and the
        // successful-refresh watermark advances.
        let state = seeded_state(&[("alpha", "old-alpha", 100), ("gamma", "old-gamma", 100)]);
        let n = install_list_refresh(
            &state,
            &[
                (
                    "id-a".to_string(),
                    "alpha".to_string(),
                    "new-alpha".to_string(),
                ),
                (
                    "id-d".to_string(),
                    "delta".to_string(),
                    "fresh-delta".to_string(),
                ),
            ],
            200,
        );
        assert_eq!(n, 2);
        let secrets = state.secrets.read().unwrap();
        assert_eq!(
            secrets.get("alpha"),
            Some(&CachedSecret {
                id: "id-a".to_string(),
                value: "new-alpha".to_string(),
                fetched_at_unix: 200
            })
        );
        assert!(!secrets.contains_key("gamma"));
        assert_eq!(secrets.get("delta").map(|c| c.fetched_at_unix), Some(200));
        assert_eq!(*state.last_refresh.read().unwrap(), Some(200));
    }

    #[test]
    fn failed_or_partial_list_refresh_applies_nothing() {
        // Oracle (#5504): if the upstream list is malformed/partial,
        // neither cached values nor the successful-refresh watermark
        // change. A valid complete list may evict a delisted name, but
        // only after every entry has validated.
        let state = seeded_state(&[("alpha", "old-alpha", 100), ("beta", "old-beta", 100)]);
        let partial = serde_json::json!([
            {"id": "id-a", "key": "alpha", "value": "new-alpha"},
            {"id": "id-b", "key": "beta"}
        ]);

        assert!(apply_list_refresh(&state, &partial, 200).is_err());
        let secrets = state.secrets.read().unwrap();
        assert_eq!(secrets.len(), 2);
        assert_eq!(
            secrets
                .get("alpha")
                .map(|s| (s.value.as_str(), s.fetched_at_unix)),
            Some(("old-alpha", 100))
        );
        assert_eq!(
            secrets
                .get("beta")
                .map(|s| (s.value.as_str(), s.fetched_at_unix)),
            Some(("old-beta", 100))
        );
        assert_eq!(*state.last_refresh.read().unwrap(), Some(1));
    }

    #[test]
    fn secret_response_exposes_age_and_source() {
        // #6767: a rotation runbook must be able to tell whether it
        // is verifying the broker or Bitwarden — the cache response
        // names its source and the value's fetch time and age.
        let at = now_unix() - 120;
        let state = seeded_state(&[("alpha", "fixture-value-not-a-secret", at)]);
        let (status, body) = handle_request(&state, "tok", "GET", "/secret/alpha", Some("tok"));
        assert_eq!(status, 200);
        assert_eq!(body["source"], "cache");
        assert_eq!(body["fetched_at_unix"], at);
        let age = body["age_seconds"].as_u64().expect("age present");
        assert!((120..130).contains(&age), "age {age}");
    }

    #[test]
    fn rfc3339_parses_the_live_bws_revision_shape() {
        // Oracle: `bws secret list` revisionDate looks like
        // 2026-10-07T07:23:52.457179600Z (9-digit fraction, Z).
        let live = parse_rfc3339_unix("2026-10-07T07:23:52.457179600Z").expect("live shape");
        assert_eq!(live, parse_rfc3339_unix("2026-10-07T07:23:52Z").expect("plain"));
        assert_eq!(
            live,
            parse_rfc3339_unix("2026-10-07T09:23:52+02:00").expect("offset")
        );
        assert_eq!(
            parse_rfc3339_unix("2026-10-07T09:23:52+02:00").expect("offset"),
            parse_rfc3339_unix("2026-10-07T07:23:52Z").expect("z") 
        );
        // Absolute anchor (verified against `date -u -d`): 2026-01-01T00:00:00Z.
        assert_eq!(parse_rfc3339_unix("2026-01-01T00:00:00Z"), Some(1767225600));
        for bad in ["", "not-a-date", "2026-13-01T00:00:00Z", "2026-01-01 00:00:00", "2026-01-01T25:00:00Z"] {
            assert_eq!(parse_rfc3339_unix(bad), None, "reject {bad}");
        }
    }

    #[test]
    fn delta_refresh_applies_exactly_the_changed_entry() {
        // Proof (1): one secret changed upstream → exactly one entry
        // applied; the untouched entry keeps its old value AND its
        // old fetch time; the delisted id is evicted.
        let state = seeded_state_ids(&[
            ("id-a", "alpha", "old-alpha", 100),
            ("id-b", "beta", "old-beta", 100),
            ("id-g", "gamma", "old-gamma", 100),
        ]);
        let entries = vec![
            ("id-a".to_string(), "alpha".to_string(), "new-alpha-upstream".to_string(), Some(900)),
            ("id-b".to_string(), "beta".to_string(), "new-beta".to_string(), Some(1100)),
        ];
        let out = apply_delta_refresh(&state, &entries, Some(1000), 2000, 1940);
        assert_eq!(out, DeltaOutcome { applied: 1, deleted: 1, total: 2 });
        let secrets = state.secrets.read().unwrap();
        // Below-watermark entry: value and fetch time untouched.
        assert_eq!(
            secrets.get("alpha").map(|c| (c.value.as_str(), c.fetched_at_unix)),
            Some(("old-alpha", 100))
        );
        assert_eq!(
            secrets.get("beta").map(|c| (c.value.as_str(), c.fetched_at_unix)),
            Some(("new-beta", 2000))
        );
        assert!(!secrets.contains_key("gamma"));
        // Single quantity: the watermark advances to start-minus-skew.
        assert_eq!(*state.last_refresh.read().unwrap(), Some(1940));
    }

    #[test]
    fn delta_refresh_with_no_changes_is_a_noop_beyond_the_list_call() {
        // Proof (2): nothing newer than the watermark → zero applies,
        // zero evictions, cached fetch times bit-identical; the
        // watermark still advances to the new refresh start-minus-skew.
        let state = seeded_state_ids(&[
            ("id-a", "alpha", "old-alpha", 100),
            ("id-b", "beta", "old-beta", 100),
        ]);
        let entries = vec![
            ("id-a".to_string(), "alpha".to_string(), "old-alpha".to_string(), Some(500)),
            ("id-b".to_string(), "beta".to_string(), "old-beta".to_string(), Some(900)),
        ];
        let out = apply_delta_refresh(&state, &entries, Some(1000), 2000, 1940);
        assert_eq!(out, DeltaOutcome { applied: 0, deleted: 0, total: 2 });
        let secrets = state.secrets.read().unwrap();
        assert_eq!(
            secrets.get("alpha").map(|c| (c.value.as_str(), c.fetched_at_unix)),
            Some(("old-alpha", 100))
        );
        assert_eq!(
            secrets.get("beta").map(|c| (c.value.as_str(), c.fetched_at_unix)),
            Some(("old-beta", 100))
        );
        assert_eq!(*state.last_refresh.read().unwrap(), Some(1940));
    }

    #[test]
    fn delta_refresh_entry_without_a_stamp_is_applied_conservatively() {
        // A missing/unparseable revisionDate is not a validation
        // failure and never skips the entry — it may have changed.
        let list = serde_json::json!([
            {"id": "id-a", "key": "alpha", "value": "v-a"},
            {"id": "id-b", "key": "beta", "value": "v-b", "revisionDate": "not-a-date"}
        ]);
        let entries = parse_delta_entries(&list).expect("stamps never fail validation");
        assert_eq!(entries[0].3, None);
        assert_eq!(entries[1].3, None);
        let state = seeded_state_ids(&[("id-a", "alpha", "old-a", 100)]);
        let out = apply_delta_refresh(&state, &entries, Some(1000), 2000, 1940);
        assert_eq!(out.applied, 2);
        assert_eq!(
            state.secrets.read().unwrap().get("alpha").map(|c| c.value.as_str()),
            Some("v-a")
        );
    }

    #[test]
    fn delta_refresh_retains_unknown_id_entries_it_cannot_prove_absent() {
        // Read-through installs (#6275) carry no Bitwarden id; the
        // id-set diff cannot prove them absent, so they survive a
        // refresh that does not list their name.
        let state = seeded_state(&[("rt-name", "rt-value", 100)]);
        let entries = vec![(
            "id-a".to_string(),
            "alpha".to_string(),
            "v-a".to_string(),
            Some(500),
        )];
        let out = apply_delta_refresh(&state, &entries, Some(1000), 2000, 1940);
        assert_eq!(out.deleted, 0);
        assert!(state.secrets.read().unwrap().contains_key("rt-name"));
    }

    #[test]
    fn malformed_delta_list_applies_nothing_including_the_watermark() {
        // Proof (#5504) at the delta gate: a partial list fails
        // before the cache is touched — values, fetch times, and the
        // watermark are all bit-identical afterwards.
        let state = seeded_state_ids(&[("id-a", "alpha", "old-alpha", 100)]);
        *state.last_refresh.write().unwrap() = Some(1000);
        let partial = serde_json::json!([
            {"id": "id-a", "key": "alpha", "value": "new-alpha", "revisionDate": "2026-01-01T00:00:00Z"},
            {"id": "id-b", "key": "beta"}
        ]);
        assert!(parse_delta_entries(&partial).is_err());
        let secrets = state.secrets.read().unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(
            secrets.get("alpha").map(|c| (c.value.as_str(), c.fetched_at_unix)),
            Some(("old-alpha", 100))
        );
        assert_eq!(*state.last_refresh.read().unwrap(), Some(1000));
    }

    #[test]
    fn skew_window_change_is_still_applied_on_the_next_refresh() {
        // Proof (4, skew): the watermark is refresh-start minus
        // WATERMARK_SKEW_S, so an entry revised DURING the refresh
        // window (after start, before the list lands) still reads as
        // newer on the following tick.
        assert_eq!(WATERMARK_SKEW_S, 60);
        let start = 2000u64;
        let watermark_new = start - WATERMARK_SKEW_S;
        let state = seeded_state_ids(&[("id-a", "alpha", "old-a", 100)]);
        // Entry revised at 1990: inside the skew window behind start.
        let entries = vec![(
            "id-a".to_string(),
            "alpha".to_string(),
            "mid-refresh-value".to_string(),
            entry_revision_unix(&serde_json::json!({"revisionDate": "1970-01-01T00:33:10Z"})),
        )];
        assert_eq!(entries[0].3, Some(1990));
        let out = apply_delta_refresh(&state, &entries, Some(1000), 2100, watermark_new);
        assert_eq!(out.applied, 1);
        assert_eq!(*state.last_refresh.read().unwrap(), Some(watermark_new));
        // And the NEXT refresh, filtering against the persisted
        // watermark, still sees it as newer (1990 > 1940): re-apply
        // is idempotent, never a skip.
        let out2 = apply_delta_refresh(&state, &entries, Some(watermark_new), 2200, 2140);
        assert_eq!(out2.applied, 1);
        assert_eq!(
            state.secrets.read().unwrap().get("alpha").map(|c| c.value.as_str()),
            Some("mid-refresh-value")
        );
    }

    #[test]
    fn watermark_round_trips_through_the_file_atomically() {
        // Proof (3/4, durability): persist + reload returns the exact
        // watermark; a torn write can never leave a half file behind
        // (write-temp-then-rename) — simulated here by asserting the
        // file bytes are exactly the decimal form.
        let dir = std::env::temp_dir().join(format!("wm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("watermark");
        // SAFETY: the watermark path override is test-only and no
        // other test in this module reads it concurrently.
        unsafe {
            std::env::set_var(
                "SECRETS_BROKER_WATERMARK_FILE",
                path.to_string_lossy().to_string(),
            );
        }
        persist_watermark(1940).expect("persist");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1940\n");
        assert_eq!(load_watermark(), Some(1940));
        // A garbage file is a cold start, never a crash.
        std::fs::write(&path, "torn-half").unwrap();
        assert_eq!(load_watermark(), None);
        std::fs::remove_dir_all(&dir).ok();
        unsafe {
            std::env::remove_var("SECRETS_BROKER_WATERMARK_FILE");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// PBT (#6568/#5504, testing doctrine): the strict list parser
        /// succeeds iff every generated entry has string `id`, `key`,
        /// and `value` fields, and on success output exactly preserves
        /// those triples in order. Oracle: well-formedness recomputed
        /// from the generated payload itself.
        #[test]
        fn parse_list_entries_succeeds_iff_every_entry_is_well_formed(
            entries in proptest::collection::vec(
                (0u8..3, 0u8..3, 0u8..3, "[a-z]{0,6}", "[a-z]{0,6}", "[a-z]{0,6}"),
                0..12,
            )
        ) {
            let list = serde_json::Value::Array(
                entries
                    .into_iter()
                    .map(|(id_kind, key_kind, value_kind, id_s, key_s, value_s)| {
                        let mut obj = serde_json::Map::new();
                        match id_kind {
                            1 => {
                                obj.insert("id".to_string(), serde_json::json!(id_s));
                            }
                            2 => {
                                obj.insert("id".to_string(), serde_json::json!(42));
                            }
                            _ => {}
                        }
                        match key_kind {
                            1 => {
                                obj.insert("key".to_string(), serde_json::json!(key_s));
                            }
                            2 => {
                                obj.insert("key".to_string(), serde_json::json!(true));
                            }
                            _ => {}
                        }
                        match value_kind {
                            1 => {
                                obj.insert("value".to_string(), serde_json::json!(value_s));
                            }
                            2 => {
                                obj.insert("value".to_string(), serde_json::json!(true));
                            }
                            _ => {}
                        }
                        serde_json::Value::Object(obj)
                    })
                    .collect(),
            );
            let expected: Option<Vec<(String, String, String)>> = list
                .as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    match (
                        s.get("id").and_then(|v| v.as_str()),
                        s.get("key").and_then(|v| v.as_str()),
                        s.get("value").and_then(|v| v.as_str()),
                    ) {
                        (Some(i), Some(k), Some(v)) => {
                            Some((i.to_string(), k.to_string(), v.to_string()))
                        }
                        _ => None,
                    }
                })
                .collect();
            match expected {
                Some(pairs) => prop_assert_eq!(parse_list_entries(&list).unwrap(), pairs),
                None => prop_assert!(parse_list_entries(&list).is_err()),
            }
        }
    }

    #[test]
    fn ttl_unset_uses_the_documented_default() {
        let parsed = parse_ttl_s(None).expect("unset is valid");
        assert_eq!(parsed.ttl_s, DEFAULT_TTL_S);
        assert_eq!(parsed.warning, None);
    }

    #[test]
    fn ttl_valid_values_are_honored() {
        for (raw, want) in [("900", 900), ("60", 60), ("30", MIN_TTL_S), (" 120 ", 120)] {
            let parsed = parse_ttl_s(Some(raw)).expect("valid value");
            assert_eq!(parsed.ttl_s, want, "raw={raw:?}");
            assert_eq!(parsed.warning, None);
        }
    }

    #[test]
    fn ttl_zero_and_below_minimum_are_refused() {
        // #5860: TTL_S=0 spun the refresh loop at ~110 full vault
        // refreshes/sec. The refusal names the variable and the value.
        for raw in ["0", "1", "29"] {
            let err = parse_ttl_s(Some(raw)).expect_err("below-minimum must refuse");
            assert!(err.contains("SECRETS_BROKER_TTL_S"), "error: {err}");
            assert!(err.contains(raw), "error: {err}");
        }
    }

    fn sh_runner(timeout: Duration) -> super::BwsRunner {
        super::BwsRunner {
            token: "tok".to_string(),
            timeout,
        }
    }

    #[test]
    fn bws_child_that_never_exits_fails_at_the_deadline() {
        // #10871 oracle: a child that never exits must come back as
        // an error near its deadline, not block its caller forever.
        use fleet_secrets::CommandRunner;
        let started = std::time::Instant::now();
        let err = sh_runner(Duration::from_millis(300))
            .run("/bin/sh", &["-c", "exec sleep 600"])
            .expect_err("a hung child must fail");
        assert!(err.contains("timed out"), "error: {err}");
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn bws_child_within_the_deadline_returns_its_stdout() {
        use fleet_secrets::CommandRunner;
        let out = sh_runner(Duration::from_secs(20))
            .run("/bin/sh", &["-c", "printf '{\"value\":\"x\"}'"])
            .expect("a prompt child succeeds");
        assert_eq!(out, "{\"value\":\"x\"}");
    }

    #[test]
    fn bws_child_failure_reports_only_the_exit_status() {
        use fleet_secrets::CommandRunner;
        let err = sh_runner(Duration::from_secs(20))
            .run("/bin/sh", &["-c", "echo vault-material >&2; exit 3"])
            .expect_err("a non-zero exit is an error");
        assert!(!err.contains("vault-material"), "error: {err}");
    }

    #[test]
    fn ttl_garbage_defaults_with_a_warning_naming_the_value() {
        for raw in ["abc", "60s", "-5", ""] {
            let parsed = parse_ttl_s(Some(raw)).expect("garbage falls back, never refuses");
            assert_eq!(parsed.ttl_s, DEFAULT_TTL_S, "raw={raw:?}");
            let warning = parsed.warning.expect("the fallback must not be silent");
            assert!(
                warning.contains("SECRETS_BROKER_TTL_S"),
                "warning: {warning}"
            );
            assert!(warning.contains(raw), "warning: {warning}");
        }
    }
}
