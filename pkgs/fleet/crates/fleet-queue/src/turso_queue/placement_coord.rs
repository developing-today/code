//! turso_queue::placement_coord — the central coordinator for fleet
//! placement dispatch (#13765).
//!
//! The queue backend's half of the placement coordinator: atomic
//! slot claims, the cross-seat true load view, the fleet-wide
//! Blacksmith budget, and the cross-seat waitlist. The client half
//! lives in `crate::placement::coordinator` and reaches these verbs
//! over the bounded broker roundtrip; when the coordinator is
//! unreachable the client dispatches exactly as before (singular
//! fallback), so every verb here is allowed to fail from the
//! client's point of view.
//!
//! Atomicity follows the house pattern (single conditional UPDATEs
//! whose affected-row count decides the winner — task claim,
//! `turso_queue/claim.rs`): the slot gate and the budget reserve
//! are each one conditional UPDATE, so two seats racing for the
//! last slot (or the last free minute) cannot both win. libSQL is
//! single-writer; the loser's UPDATE matches no row.
//!
//! Owner rows are seeded lazily (INSERT OR IGNORE on first claim
//! for that owner, then every verb re-syncs the cap from the
//! embedded registry — the registry is the authority, never the
//! caller), and the coordinator sweep (`coordinator_sweep`,
//! driven by `queue_broker::run`) refreshes the GitHub counts and
//! recomputes `claims_active` from the claims table, so a crashed
//! or partitioned broker heals instead of leaking slots.

use super::*;

/// How long a granted slot (`claimed`) waits for its dispatch to
/// confirm before the sweep reclaims it.
pub(crate) const CLAIM_TTL_SECS: i64 = 300;
/// How long a waitlist promotion (`granted`) waits for the seat to
/// notice before the sweep reclaims it.
pub(crate) const GRANT_TTL_SECS: i64 = 120;

/// The coordinator's true load view per owner: the registry cap
/// plus the last sweep's GitHub counts plus the slots this table
/// has granted since. `claims_active` counts `claimed` + `granted`
/// + `confirmed` rows; `confirmed` rows stop counting once their
/// run is observed live (GitHub then carries them) — the sweep
/// owns that recomputation.
pub(crate) const PLACEMENT_COORD_DDL_OWNER: &str = "CREATE TABLE IF NOT EXISTS placement_owner_state (\
    owner TEXT PRIMARY KEY, cap INTEGER NOT NULL DEFAULT 0, \
    gh_running INTEGER NOT NULL DEFAULT 0, gh_queued INTEGER NOT NULL DEFAULT 0, \
    load_read_at TEXT, load_source TEXT, claims_active INTEGER NOT NULL DEFAULT 0, \
    updated_at TEXT NOT NULL)";
/// One row per slot request. States: `claimed` (slot held, confirm
/// within [`CLAIM_TTL_SECS`]), `queued` (waitlist, FIFO per
/// owner), `granted` (promoted by the sweep, confirm within
/// [`GRANT_TTL_SECS`]), `confirmed` (dispatched), `released`
/// (given up), `expired` (TTL lapsed, slot reclaimed), `settled`
/// (terminal Blacksmith use converted to `used_minutes`).
pub(crate) const PLACEMENT_COORD_DDL_CLAIMS: &str = "CREATE TABLE IF NOT EXISTS placement_claims (\
    handle TEXT PRIMARY KEY, owner TEXT NOT NULL, surface TEXT NOT NULL, \
    kind TEXT NOT NULL, seat TEXT NOT NULL, state TEXT NOT NULL, \
    run_id INTEGER, est_minutes REAL NOT NULL DEFAULT 0, budget_month TEXT, \
    created_at TEXT NOT NULL, expires_at TEXT, updated_at TEXT NOT NULL)";
/// Fleet-wide Blacksmith budget per month and surface: minutes
/// `reserved` at claim, converted to `used` at settle. The budget
/// itself (`budget_minutes`) is re-synced from the embedded
/// registry on every claim — the registry is the authority.
pub(crate) const PLACEMENT_COORD_DDL_BUDGETS: &str = "CREATE TABLE IF NOT EXISTS placement_budgets (\
    month TEXT NOT NULL, surface TEXT NOT NULL, budget_minutes REAL NOT NULL DEFAULT 0, \
    reserved_minutes REAL NOT NULL DEFAULT 0, used_minutes REAL NOT NULL DEFAULT 0, \
    updated_at TEXT NOT NULL, PRIMARY KEY (month, surface))";
/// FIFO promotion order is `(owner, state, created_at)`.
pub(crate) const PLACEMENT_COORD_DDL_CLAIMS_IDX: &str =
    "CREATE INDEX IF NOT EXISTS placement_claims_owner_state \
    ON placement_claims(owner, state, created_at)";

/// The embedded surface registry — the same file the placement
/// client decides from (`pkgs/fleet/fleet-surfaces.json`). Owner,
/// kind, cap, and budget are always derived here; a caller that
/// could name its own cap could grant itself slots.
const COORD_REGISTRY: &str = include_str!("../../../../fleet-surfaces.json");

/// What a claim needs from the registry: the surface's owner and
/// kind, the owner's concurrent-jobs cap, and the Blacksmith
/// budget (free + paid-cap minutes) for Blacksmith surfaces.
struct SurfaceSpec {
    owner: String,
    kind: String,
    cap: i64,
    budget_minutes: f64,
}

fn coord_registry() -> Result<Value, String> {
    let v: Value =
        serde_json::from_str(COORD_REGISTRY).map_err(|e| format!("embedded registry invalid: {e}"))?;
    if v.get("surfaces").and_then(|s| s.as_array()).is_none() {
        return Err("embedded registry has no surfaces array".to_string());
    }
    Ok(v)
}

fn surface_spec(registry: &Value, id: &str) -> Result<SurfaceSpec, String> {
    let surface = registry
        .get("surfaces")
        .and_then(|s| s.as_array())
        .and_then(|ss| ss.iter().find(|s| s.get("id").and_then(|v| v.as_str()) == Some(id)))
        .ok_or_else(|| format!("unknown surface {id}"))?;
    let owner = surface
        .get("owner")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if owner.is_empty() {
        return Err(format!("surface {id} has no owner"));
    }
    let kind = surface
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let cap = registry
        .get("owners")
        .and_then(|o| o.get(&owner))
        .and_then(|o| o.get("concurrent_jobs_cap"))
        .and_then(|c| c.as_i64())
        .unwrap_or(0);
    let budget_minutes = surface
        .get("budget")
        .map(|b| {
            b.get("free_minutes_monthly").and_then(|v| v.as_f64()).unwrap_or(0.0)
                + b.get("paid_cap_minutes").and_then(|v| v.as_f64()).unwrap_or(0.0)
        })
        .unwrap_or(0.0);
    Ok(SurfaceSpec {
        owner,
        kind,
        cap,
        budget_minutes,
    })
}

/// This month's budget bucket (`YYYY-MM`, UTC).
fn budget_month() -> String {
    now_iso()[..7].to_string()
}

/// Seed the owner's row if absent, then re-sync the cap from the
/// registry (the registry may have changed since the row was
/// seeded; a stale cap gates wrong).
fn ensure_owner(t: &TursoBackend, owner: &str, cap: i64) -> Result<(), TErr> {
    let now = now_iso();
    t.exec(
        "INSERT OR IGNORE INTO placement_owner_state \
        (owner, cap, updated_at) VALUES (?, ?, ?)",
        &[Arg::Text(owner.to_string()), Arg::Int(cap), Arg::Text(now.clone())],
    )?;
    t.exec(
        "UPDATE placement_owner_state SET cap = ?, updated_at = ? WHERE owner = ?",
        &[Arg::Int(cap), Arg::Text(now), Arg::Text(owner.to_string())],
    )?;
    Ok(())
}

/// Seed the month's budget row if absent, then re-sync the budget
/// from the registry.
fn ensure_budget(t: &TursoBackend, month: &str, surface: &str, budget: f64) -> Result<(), TErr> {
    let now = now_iso();
    t.exec(
        "INSERT OR IGNORE INTO placement_budgets \
        (month, surface, budget_minutes, updated_at) VALUES (?, ?, ?, ?)",
        &[
            Arg::Text(month.to_string()),
            Arg::Text(surface.to_string()),
            Arg::Float(budget),
            Arg::Text(now.clone()),
        ],
    )?;
    t.exec(
        "UPDATE placement_budgets SET budget_minutes = ?, updated_at = ? \
        WHERE month = ? AND surface = ?",
        &[
            Arg::Float(budget),
            Arg::Text(now),
            Arg::Text(month.to_string()),
            Arg::Text(surface.to_string()),
        ],
    )?;
    Ok(())
}

/// The owner's true view: cap, GitHub counts, active claims, and
/// occupancy (the admission sum). None when the owner has never
/// been seen — the sweep seeds every registry owner on its first
/// pass, so None only means "no sweep yet and no claim yet".
fn owner_view(t: &TursoBackend, owner: &str) -> Result<Value, TErr> {
    let rows = t.query_objs(
        "SELECT owner, cap, gh_running, gh_queued, load_read_at, load_source, \
        claims_active FROM placement_owner_state WHERE owner = ?",
        &[Arg::Text(owner.to_string())],
        &[],
    )?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(Value::Null);
    };
    let cap = row.get("cap").and_then(|v| v.as_i64()).unwrap_or(0);
    let running = row.get("gh_running").and_then(|v| v.as_i64()).unwrap_or(0);
    let queued = row.get("gh_queued").and_then(|v| v.as_i64()).unwrap_or(0);
    let active = row.get("claims_active").and_then(|v| v.as_i64()).unwrap_or(0);
    Ok(json!({
        "owner": owner,
        "cap": cap,
        "gh_running": running,
        "gh_queued": queued,
        "claims_active": active,
        "occupancy": running + queued + active,
        "load_read_at": row.get("load_read_at").cloned().unwrap_or(Value::Null),
        "load_source": row.get("load_source").cloned().unwrap_or(Value::Null),
    }))
}

fn claim_row(t: &TursoBackend, handle: &str) -> Result<Value, TErr> {
    let rows = t.query_objs(
        "SELECT handle, owner, surface, kind, seat, state, run_id, est_minutes, \
        budget_month, created_at, expires_at, updated_at \
        FROM placement_claims WHERE handle = ?",
        &[Arg::Text(handle.to_string())],
        &[],
    )?;
    Ok(rows.into_iter().next().unwrap_or(Value::Null))
}

/// Waitlist position of a `queued` claim: 1-based FIFO among the
/// owner's queued claims by creation time (handle breaks ties
/// within one timestamp tick).
fn queue_position(t: &TursoBackend, owner: &str, created_at: &str, handle: &str) -> Result<i64, TErr> {
    let rows = t.query_objs(
        "SELECT COUNT(*) AS n FROM placement_claims \
        WHERE owner = ? AND state = 'queued' \
        AND (created_at < ? OR (created_at = ? AND handle <= ?))",
        &[
            Arg::Text(owner.to_string()),
            Arg::Text(created_at.to_string()),
            Arg::Text(created_at.to_string()),
            Arg::Text(handle.to_string()),
        ],
        &[],
    )?;
    Ok(rows
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1))
}

/// Claim a slot: the coordinator's admission gate. Derives owner,
/// kind, cap, and budget from the embedded registry, then (for
/// Blacksmith) reserves the estimate fleet-wide, then takes the
/// slot through the single conditional UPDATE — one row affected
/// means the slot is held (`claimed`, confirm within
/// [`CLAIM_TTL_SECS`]); no row means the owner is full and the
/// claim waits (`queued`, with its FIFO position). A spent budget
/// refuses terminally (`refused`) with no claim row.
pub(crate) fn verb_placement_claim(t: &TursoBackend, req: &Value) -> Value {
    let Some(handle) = req_str(req, "handle").filter(|h| !h.is_empty()) else {
        return fail(2, "handle is required".to_string());
    };
    let handle = handle.to_string();
    let Some(surface) = req_str(req, "surface").filter(|s| !s.is_empty()) else {
        return fail(2, "surface is required".to_string());
    };
    let surface = surface.to_string();
    let Some(seat) = req_str(req, "seat").filter(|s| !s.is_empty()) else {
        return fail(2, "seat is required".to_string());
    };
    let seat = seat.to_string();
    let est = req.get("est_minutes").and_then(|v| v.as_f64()).unwrap_or(10.0);
    if !est.is_finite() || est < 0.0 {
        return fail(2, "est_minutes must be a non-negative number".to_string());
    }
    let allow_paid = req.get("allow_paid").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if !allow_paid.is_finite() || allow_paid < 0.0 {
        return fail(2, "allow_paid must be a non-negative number".to_string());
    }
    let work = (|| -> Result<Value, TErr> {
        let registry = coord_registry().map_err(|e| e.to_string())?;
        let spec = surface_spec(&registry, &surface).map_err(|e| e)?;
        ensure_owner(t, &spec.owner, spec.cap)?;
        let now = now_iso();
        // Blacksmith first: reserve the estimate against the
        // fleet-wide budget — one conditional UPDATE, so two seats
        // racing for the last free minute cannot both win. A spent
        // budget refuses terminally: no claim row, nothing to poll.
        let month;
        if spec.kind == "blacksmith" {
            month = budget_month();
            ensure_budget(t, &month, &surface, spec.budget_minutes)?;
            let reserve = t.exec(
                "UPDATE placement_budgets \
                SET reserved_minutes = reserved_minutes + ?, updated_at = ? \
                WHERE month = ? AND surface = ? \
                AND used_minutes + reserved_minutes + ? <= budget_minutes + ?",
                &[
                    Arg::Float(est),
                    Arg::Text(now.clone()),
                    Arg::Text(month.clone()),
                    Arg::Text(surface.clone()),
                    Arg::Float(est),
                    Arg::Float(allow_paid),
                ],
            )?;
            if reserve.affected == 0 {
                return Ok(json!({
                    "handle": handle,
                    "status": "refused",
                    "reason": "budget-exhausted",
                    "surface": surface,
                    "owner": spec.owner,
                }));
            }
        } else {
            month = String::new();
        }
        // Then the slot gate — one conditional UPDATE, the house
        // atomicity pattern: exactly one racer for the last slot
        // sees its counter move.
        let gate = t.exec(
            "UPDATE placement_owner_state \
            SET claims_active = claims_active + 1, updated_at = ? \
            WHERE owner = ? AND gh_running + gh_queued + claims_active < cap",
            &[Arg::Text(now.clone()), Arg::Text(spec.owner.clone())],
        )?;
        if gate.affected == 1 {
            let expires = after_iso(CLAIM_TTL_SECS);
            t.exec(
                "INSERT INTO placement_claims \
                (handle, owner, surface, kind, seat, state, est_minutes, budget_month, \
                created_at, expires_at, updated_at) \
                VALUES (?, ?, ?, ?, ?, 'claimed', ?, ?, ?, ?, ?)",
                &[
                    Arg::Text(handle.clone()),
                    Arg::Text(spec.owner.clone()),
                    Arg::Text(surface.clone()),
                    Arg::Text(spec.kind.clone()),
                    Arg::Text(seat.clone()),
                    Arg::Float(est),
                    arg_str(if month.is_empty() { None } else { Some(&month) }),
                    Arg::Text(now.clone()),
                    Arg::Text(expires.clone()),
                    Arg::Text(now.clone()),
                ],
            )?;
            Ok(json!({
                "handle": handle,
                "status": "claimed",
                "surface": surface,
                "owner": spec.owner,
                "seat": seat,
                "expires_at": expires,
                "owner_load": owner_view(t, &spec.owner)?,
            }))
        } else {
            t.exec(
                "INSERT INTO placement_claims \
                (handle, owner, surface, kind, seat, state, est_minutes, budget_month, \
                created_at, expires_at, updated_at) \
                VALUES (?, ?, ?, ?, ?, 'queued', ?, ?, ?, NULL, ?)",
                &[
                    Arg::Text(handle.clone()),
                    Arg::Text(spec.owner.clone()),
                    Arg::Text(surface.clone()),
                    Arg::Text(spec.kind.clone()),
                    Arg::Text(seat.clone()),
                    Arg::Float(est),
                    arg_str(if month.is_empty() { None } else { Some(&month) }),
                    Arg::Text(now.clone()),
                    Arg::Text(now.clone()),
                ],
            )?;
            let position = queue_position(t, &spec.owner, &now, &handle)?;
            Ok(json!({
                "handle": handle,
                "status": "queued",
                "surface": surface,
                "owner": spec.owner,
                "seat": seat,
                "position": position,
                "owner_load": owner_view(t, &spec.owner)?,
            }))
        }
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => {
            if e.starts_with("unknown surface ") || e.ends_with("has no owner") {
                fail(2, e)
            } else {
                terr(e)
            }
        }
    }
}

/// Confirm a held slot after dispatch: `claimed`/`granted` become
/// `confirmed`, carrying the GitHub run id when the dispatch
/// correlated one. Anything else (unknown handle, expired,
/// released, already settled) confirms nothing — the seat must
/// claim again.
pub(crate) fn verb_placement_confirm(t: &TursoBackend, req: &Value) -> Value {
    let Some(handle) = req_str(req, "handle").filter(|h| !h.is_empty()) else {
        return fail(2, "handle is required".to_string());
    };
    let handle = handle.to_string();
    let run_id = req.get("run_id").and_then(|v| v.as_i64());
    let work = (|| -> Result<Value, TErr> {
        let now = now_iso();
        let upd = t.exec(
            "UPDATE placement_claims SET state = 'confirmed', \
            run_id = COALESCE(?, run_id), expires_at = NULL, updated_at = ? \
            WHERE handle = ? AND state IN ('claimed', 'granted')",
            &[
                run_id.map(Arg::Int).unwrap_or(Arg::Null),
                Arg::Text(now),
                Arg::Text(handle.clone()),
            ],
        )?;
        if upd.affected == 1 {
            Ok(json!({"handle": handle, "confirmed": true, "state": "confirmed", "run_id": run_id}))
        } else {
            Ok(json!({"handle": handle, "confirmed": false,
                "reason": "claim is not confirmable (unknown, expired, or already settled)"}))
        }
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

/// Release a slot: `claimed`/`granted` give the slot back (and, for
/// Blacksmith, unreserve the estimate); `confirmed` with actual
/// minutes converts the reservation to use (`settled`); `confirmed`
/// without actuals just closes the row; terminal states are already
/// gone and release idempotently.
pub(crate) fn verb_placement_release(t: &TursoBackend, req: &Value) -> Value {
    let Some(handle) = req_str(req, "handle").filter(|h| !h.is_empty()) else {
        return fail(2, "handle is required".to_string());
    };
    let handle = handle.to_string();
    let actual = req.get("actual_minutes").and_then(|v| v.as_f64());
    if let Some(a) = actual {
        if !a.is_finite() || a < 0.0 {
            return fail(2, "actual_minutes must be a non-negative number".to_string());
        }
    }
    let work = (|| -> Result<Value, TErr> {
        let row = claim_row(t, &handle)?;
        let state = row.get("state").and_then(|v| v.as_str()).unwrap_or("");
        let now = now_iso();
        match state {
            "" => Ok(json!({"handle": handle, "released": false, "reason": "unknown claim"})),
            "released" | "expired" | "settled" => {
                Ok(json!({"handle": handle, "released": true, "state": state, "note": "already terminal"}))
            }
            "claimed" | "granted" | "queued" => {
                let owner = row.get("owner").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let est = row.get("est_minutes").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let month = row.get("budget_month").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let surface = row.get("surface").and_then(|v| v.as_str()).unwrap_or("").to_string();
                t.exec(
                    "UPDATE placement_claims SET state = 'released', expires_at = NULL, \
                    updated_at = ? WHERE handle = ?",
                    &[Arg::Text(now.clone()), Arg::Text(handle.clone())],
                )?;
                // Queued claims never held a slot — only the
                // reservation unwinds. Held slots free the counter.
                if state != "queued" && !owner.is_empty() {
                    t.exec(
                        "UPDATE placement_owner_state \
                        SET claims_active = CASE WHEN claims_active > 0 \
                        THEN claims_active - 1 ELSE 0 END, updated_at = ? WHERE owner = ?",
                        &[Arg::Text(now.clone()), Arg::Text(owner.clone())],
                    )?;
                }
                if kind == "blacksmith" && !month.is_empty() {
                    t.exec(
                        "UPDATE placement_budgets \
                        SET reserved_minutes = CASE WHEN reserved_minutes > ? \
                        THEN reserved_minutes - ? ELSE 0 END, updated_at = ? \
                        WHERE month = ? AND surface = ?",
                        &[
                            Arg::Float(est),
                            Arg::Float(est),
                            Arg::Text(now.clone()),
                            Arg::Text(month.clone()),
                            Arg::Text(surface.clone()),
                        ],
                    )?;
                }
                Ok(json!({"handle": handle, "released": true, "state": "released"}))
            }
            "confirmed" => {
                let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let est = row.get("est_minutes").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let month = row.get("budget_month").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let surface = row.get("surface").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if kind == "blacksmith" && !month.is_empty() {
                    if let Some(a) = actual {
                        t.exec(
                            "UPDATE placement_claims SET state = 'settled', updated_at = ? \
                            WHERE handle = ?",
                            &[Arg::Text(now.clone()), Arg::Text(handle.clone())],
                        )?;
                        t.exec(
                            "UPDATE placement_budgets \
                            SET reserved_minutes = CASE WHEN reserved_minutes > ? \
                            THEN reserved_minutes - ? ELSE 0 END, \
                            used_minutes = used_minutes + ?, updated_at = ? \
                            WHERE month = ? AND surface = ?",
                            &[
                                Arg::Float(est),
                                Arg::Float(est),
                                Arg::Float(a),
                                Arg::Text(now.clone()),
                                Arg::Text(month.clone()),
                                Arg::Text(surface.clone()),
                            ],
                        )?;
                        Ok(json!({"handle": handle, "released": true, "state": "settled",
                            "actual_minutes": a}))
                    } else {
                        t.exec(
                            "UPDATE placement_claims SET state = 'released', updated_at = ? \
                            WHERE handle = ?",
                            &[Arg::Text(now.clone()), Arg::Text(handle.clone())],
                        )?;
                        t.exec(
                            "UPDATE placement_budgets \
                            SET reserved_minutes = CASE WHEN reserved_minutes > ? \
                            THEN reserved_minutes - ? ELSE 0 END, updated_at = ? \
                            WHERE month = ? AND surface = ?",
                            &[
                                Arg::Float(est),
                                Arg::Float(est),
                                Arg::Text(now.clone()),
                                Arg::Text(month.clone()),
                                Arg::Text(surface.clone()),
                            ],
                        )?;
                        Ok(json!({"handle": handle, "released": true, "state": "released",
                            "note": "confirmed Blacksmith use closed without actuals; estimate unreserved"}))
                    }
                } else {
                    // Non-Blacksmith confirmed use holds no
                    // reservation; the counter is the sweep's to
                    // recompute (its run may still be observed
                    // live), so the row just closes.
                    t.exec(
                        "UPDATE placement_claims SET state = 'released', updated_at = ? \
                        WHERE handle = ?",
                        &[Arg::Text(now.clone()), Arg::Text(handle.clone())],
                    )?;
                    Ok(json!({"handle": handle, "released": true, "state": "released"}))
                }
            }
            other => Ok(json!({"handle": handle, "released": false,
                "reason": format!("claim is in state {other}")})),
        }
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

/// One claim plus the owner's true view — resolvable by any seat,
/// so a session committed as `queued` on one seat can be polled
/// from another.
pub(crate) fn verb_placement_claim_status(t: &TursoBackend, req: &Value) -> Value {
    let Some(handle) = req_str(req, "handle").filter(|h| !h.is_empty()) else {
        return fail(2, "handle is required".to_string());
    };
    let handle = handle.to_string();
    let work = (|| -> Result<Value, TErr> {
        let row = claim_row(t, &handle)?;
        if row.is_null() {
            return Ok(json!({"handle": handle, "found": false}));
        }
        let owner = row.get("owner").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mut out = Map::new();
        out.insert("handle".to_string(), json!(handle));
        out.insert("found".to_string(), json!(true));
        out.insert("claim".to_string(), row);
        out.insert("owner_load".to_string(), owner_view(t, &owner)?);
        Ok(Value::Object(out))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

/// Every owner's true view: cap, GitHub counts, active claims,
/// occupancy, and the read's age and source. Owners are seeded
/// from the registry on every call, so a fresh broker still
/// answers the full shape (zeros until the first sweep reads
/// GitHub).
pub(crate) fn verb_placement_load(t: &TursoBackend, _req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let registry = coord_registry().map_err(|e| e.to_string())?;
        let owners = registry
            .get("owners")
            .and_then(|o| o.as_object())
            .cloned()
            .unwrap_or_default();
        let mut views = Vec::new();
        for (owner, spec) in &owners {
            let cap = spec
                .get("concurrent_jobs_cap")
                .and_then(|c| c.as_i64())
                .unwrap_or(0);
            ensure_owner(t, owner, cap)?;
            views.push(owner_view(t, owner)?);
        }
        views.sort_by(|a, b| {
            a.get("owner")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(b.get("owner").and_then(|v| v.as_str()).unwrap_or(""))
        });
        Ok(json!({"owners": views}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Coordinator worker (slice 2): the broker thread's sweep.
// ---------------------------------------------------------------------------

/// How often the broker's coordinator thread runs the sweep.
pub(crate) const COORD_SWEEP_INTERVAL_SECS: u64 = 20;

/// The sweep's GitHub surface: load counts and run states over the
/// runner-host repos — the same reads `build_ctx` makes. The
/// production implementation lives in queue_broker.rs (broker-owned
/// token, full-coverage host); tests substitute a fake.
pub(crate) trait CoordGh {
    /// (running, queued) totals for one host repo.
    fn load_counts(&self, repo: &str) -> Result<(i64, i64), String>;
    /// Live (in_progress or queued) run ids for one host repo.
    fn live_run_ids(&self, repo: &str) -> Result<Vec<i64>, String>;
    /// Terminal runs resolve to their billable minutes; live runs
    /// resolve to None. Errors leave the claim alone.
    fn run_actual_minutes(&self, repo: &str, run_id: i64) -> Result<Option<f64>, String>;
}

/// Runner-host repos per owner — the same derivation `build_ctx`
/// reads load over: github-actions and blacksmith surfaces with a
/// repo. Sorted per owner for deterministic sweeps.
fn coord_hosts(registry: &Value) -> HashMap<String, Vec<String>> {
    let mut hosts: HashMap<String, Vec<String>> = HashMap::new();
    for s in registry
        .get("surfaces")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let kind = s.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let repo = s.get("repo").and_then(|v| v.as_str()).unwrap_or("");
        let owner = s.get("owner").and_then(|v| v.as_str()).unwrap_or("");
        if (kind == "github-actions" || kind == "blacksmith") && !repo.is_empty() && !owner.is_empty()
        {
            let repos = hosts.entry(owner.to_string()).or_default();
            if !repos.contains(&repo.to_string()) {
                repos.push(repo.to_string());
            }
        }
    }
    for repos in hosts.values_mut() {
        repos.sort();
    }
    hosts
}

fn coord_cap(registry: &Value, owner: &str) -> i64 {
    registry
        .get("owners")
        .and_then(|o| o.get(owner))
        .and_then(|o| o.get("concurrent_jobs_cap"))
        .and_then(|c| c.as_i64())
        .unwrap_or(0)
}

fn coord_surface_repo(registry: &Value, surface: &str) -> String {
    registry
        .get("surfaces")
        .and_then(|s| s.as_array())
        .and_then(|ss| {
            ss.iter()
                .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(surface))
        })
        .and_then(|s| s.get("repo"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

// Placement rows and the coordinator's own stamps are the same
// `YYYY-MM-DDTHH:MM:SSZ` shape `build_ctx` parses (private there,
// mirrored here so the sweep never depends on client features).
fn coord_days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

fn coord_parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let num = |a: usize, n: usize| s.get(a..a + n)?.parse::<i64>().ok();
    let days = coord_days_from_civil(num(0, 4)?, num(5, 2)? as u32, num(8, 2)? as u32);
    Some(days * 86_400 + num(11, 2)? * 3600 + num(14, 2)? * 60 + num(17, 2)?)
}

fn coord_civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn coord_iso_from_epoch(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = coord_civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Billable minutes between two GitHub timestamps: whole minutes
/// rounded up, at least one. None when either stamp is malformed.
pub(crate) fn coord_minutes_between(start: &str, end: &str) -> Option<f64> {
    let secs = coord_parse_iso(end)? - coord_parse_iso(start)?;
    Some(1.0f64.max((secs as f64 / 60.0).ceil()))
}

/// The sweep: refresh the true load view, expire stale holds,
/// settle finished Blacksmith runs, recompute the counters from
/// the claims table (self-healing), and grant the waitlist FIFO
/// while occupancy leaves room. Every leg is failure-tolerant —
/// a GitHub outage freezes the counts it cannot read and records
/// the error, but expiry, recompute, and grants still run, and a
/// bad row never aborts the pass. Returns the pass summary (the
/// broker thread logs it).
pub(crate) fn coordinator_sweep(t: &TursoBackend, gh: &dyn CoordGh, now: &str) -> Value {
    let mut errors: Vec<Value> = Vec::new();
    let mut refreshed: i64 = 0;
    let mut expired: i64 = 0;
    let mut settled: i64 = 0;
    let mut granted: i64 = 0;
    let registry = match coord_registry() {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let hosts = coord_hosts(&registry);
    let month = now.get(..7).unwrap_or("").to_string();
    // (1) Load refresh: each owner's GitHub counts plus the live
    // run-id set (the confirmed-but-observed-live double-count
    // guard in step 4). An owner whose repos cannot be read keeps
    // its last counts — the gate still enforces on those.
    let mut live_by_owner: HashMap<String, HashSet<i64>> = HashMap::new();
    let owners: Vec<String> = {
        let mut os: Vec<String> = registry
            .get("owners")
            .and_then(|o| o.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        os.sort();
        os
    };
    for owner in &owners {
        let cap = coord_cap(&registry, owner);
        if let Err(e) = ensure_owner(t, owner, cap) {
            errors.push(json!(format!("owner {owner}: {e}")));
            continue;
        }
        let repos = hosts.get(owner).cloned().unwrap_or_default();
        if repos.is_empty() {
            continue;
        }
        let mut running: i64 = 0;
        let mut queued: i64 = 0;
        let mut live: HashSet<i64> = HashSet::new();
        let mut failed: Option<String> = None;
        for repo in &repos {
            match gh.load_counts(repo) {
                Ok((r, q)) => {
                    running += r;
                    queued += q;
                }
                Err(e) => {
                    failed = Some(format!("{repo}: {e}"));
                    break;
                }
            }
            match gh.live_run_ids(repo) {
                Ok(ids) => {
                    live.extend(ids);
                }
                Err(e) => {
                    failed = Some(format!("{repo}: {e}"));
                    break;
                }
            }
        }
        if let Some(e) = failed {
            errors.push(json!(format!("owner {owner}: {e}")));
            continue;
        }
        live_by_owner.insert(owner.clone(), live);
        let upd = t.exec(
            "UPDATE placement_owner_state SET gh_running = ?, gh_queued = ?, \
            load_read_at = ?, load_source = ?, updated_at = ? WHERE owner = ?",
            &[
                Arg::Int(running),
                Arg::Int(queued),
                Arg::Text(now.to_string()),
                Arg::Text(
                    "broker coordinator sweep (live GitHub Actions run counts over fleet runner hosts)"
                        .to_string(),
                ),
                Arg::Text(now.to_string()),
                Arg::Text(owner.clone()),
            ],
        );
        match upd {
            Ok(_) => refreshed += 1,
            Err(e) => errors.push(json!(format!("owner {owner}: {e}"))),
        }
    }
    // (2) Expiry: claimed/granted holds past their TTL give the
    // slot back, and Blacksmith holds unreserve the estimate.
    let stale = t.query_objs(
        "SELECT handle, owner, kind, est_minutes, budget_month, surface, state \
        FROM placement_claims WHERE state IN ('claimed', 'granted') \
        AND expires_at IS NOT NULL AND expires_at < ?",
        &[Arg::Text(now.to_string())],
        &[],
    );
    match stale {
        Ok(rows) => {
            for row in rows {
                let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let owner = row.get("owner").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let est = row.get("est_minutes").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let bmonth = row
                    .get("budget_month")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let surface = row.get("surface").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let step: Result<(), TErr> = (|| {
                    t.exec(
                        "UPDATE placement_claims SET state = 'expired', expires_at = NULL, \
                        updated_at = ? WHERE handle = ?",
                        &[Arg::Text(now.to_string()), Arg::Text(handle.clone())],
                    )?;
                    if !owner.is_empty() {
                        t.exec(
                            "UPDATE placement_owner_state \
                            SET claims_active = CASE WHEN claims_active > 0 \
                            THEN claims_active - 1 ELSE 0 END, updated_at = ? WHERE owner = ?",
                            &[Arg::Text(now.to_string()), Arg::Text(owner.clone())],
                        )?;
                    }
                    if kind == "blacksmith" && !bmonth.is_empty() {
                        t.exec(
                            "UPDATE placement_budgets \
                            SET reserved_minutes = CASE WHEN reserved_minutes > ? \
                            THEN reserved_minutes - ? ELSE 0 END, updated_at = ? \
                            WHERE month = ? AND surface = ?",
                            &[
                                Arg::Float(est),
                                Arg::Float(est),
                                Arg::Text(now.to_string()),
                                Arg::Text(bmonth.clone()),
                                Arg::Text(surface.clone()),
                            ],
                        )?;
                    }
                    Ok(())
                })();
                match step {
                    Ok(()) => expired += 1,
                    Err(e) => errors.push(json!(format!("expire {handle}: {e}"))),
                }
            }
        }
        Err(e) => errors.push(json!(format!("expire scan: {e}"))),
    }
    // (3) Settle: confirmed Blacksmith claims whose runs are
    // terminal convert the reservation to actual use.
    let done = t.query_objs(
        "SELECT handle, surface, run_id, est_minutes, budget_month \
        FROM placement_claims WHERE state = 'confirmed' AND kind = 'blacksmith' \
        AND run_id IS NOT NULL",
        &[],
        &[],
    );
    match done {
        Ok(rows) => {
            for row in rows {
                let handle = row.get("handle").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let surface = row.get("surface").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let run_id = row.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0);
                let est = row.get("est_minutes").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let bmonth = row
                    .get("budget_month")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let repo = coord_surface_repo(&registry, &surface);
                if repo.is_empty() || run_id <= 0 || bmonth.is_empty() {
                    continue;
                }
                match gh.run_actual_minutes(&repo, run_id) {
                    Ok(Some(actual)) => {
                        let step: Result<(), TErr> = (|| {
                            t.exec(
                                "UPDATE placement_claims SET state = 'settled', updated_at = ? \
                                WHERE handle = ?",
                                &[Arg::Text(now.to_string()), Arg::Text(handle.clone())],
                            )?;
                            t.exec(
                                "UPDATE placement_budgets \
                                SET reserved_minutes = CASE WHEN reserved_minutes > ? \
                                THEN reserved_minutes - ? ELSE 0 END, \
                                used_minutes = used_minutes + ?, updated_at = ? \
                                WHERE month = ? AND surface = ?",
                                &[
                                    Arg::Float(est),
                                    Arg::Float(est),
                                    Arg::Float(actual),
                                    Arg::Text(now.to_string()),
                                    Arg::Text(bmonth.clone()),
                                    Arg::Text(surface.clone()),
                                ],
                            )?;
                            Ok(())
                        })();
                        match step {
                            Ok(()) => settled += 1,
                            Err(e) => errors.push(json!(format!("settle {handle}: {e}"))),
                        }
                    }
                    Ok(None) => {}
                    Err(e) => errors.push(json!(format!("run {repo}/{run_id}: {e}"))),
                }
            }
        }
        Err(e) => errors.push(json!(format!("settle scan: {e}"))),
    }
    // (4) Recompute: claims_active from the claims table, so a
    // crashed or partitioned broker heals instead of leaking
    // slots. Confirmed claims whose run is observed live stop
    // counting — GitHub carries them then (no double count).
    for owner in &owners {
        let live = live_by_owner.get(owner).cloned().unwrap_or_default();
        let rows = t.query_objs(
            "SELECT state, run_id FROM placement_claims \
            WHERE owner = ? AND state IN ('claimed', 'granted', 'confirmed')",
            &[Arg::Text(owner.clone())],
            &[],
        );
        let active: i64 = match rows {
            Ok(rs) => {
                rs.iter()
                    .filter(|r| {
                        let st = r.get("state").and_then(|v| v.as_str()).unwrap_or("");
                        if st == "claimed" || st == "granted" {
                            return true;
                        }
                        if st != "confirmed" {
                            return false;
                        }
                        match r.get("run_id").and_then(|v| v.as_i64()) {
                            Some(id) => !live.contains(&id),
                            None => true,
                        }
                    })
                    .count() as i64
            }
            Err(e) => {
                errors.push(json!(format!("recompute {owner}: {e}")));
                continue;
            }
        };
        if let Err(e) = t.exec(
            "UPDATE placement_owner_state SET claims_active = ?, updated_at = ? WHERE owner = ?",
            &[
                Arg::Int(active),
                Arg::Text(now.to_string()),
                Arg::Text(owner.clone()),
            ],
        ) {
            errors.push(json!(format!("recompute {owner}: {e}")));
        }
    }
    // (5) Grants: the waitlist, FIFO per owner, while occupancy
    // (GitHub counts plus still-held claims) leaves room. The
    // head of the line blocks the owner (no skipping): a queued
    // Blacksmith claim the budget cannot cover waits.
    for owner in &owners {
        let cap = coord_cap(&registry, owner);
        loop {
            let view = match owner_view(t, owner) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(json!(format!("grant view {owner}: {e}")));
                    break;
                }
            };
            let occupancy = view.get("occupancy").and_then(|v| v.as_i64()).unwrap_or(i64::MAX);
            if occupancy >= cap {
                break;
            }
            let next = t.query_objs(
                "SELECT handle, surface, kind, est_minutes, budget_month \
                FROM placement_claims WHERE owner = ? AND state = 'queued' \
                ORDER BY created_at, handle LIMIT 1",
                &[Arg::Text(owner.clone())],
                &[],
            );
            let head = match next {
                Ok(mut rs) => {
                    if rs.is_empty() {
                        break;
                    }
                    rs.remove(0)
                }
                Err(e) => {
                    errors.push(json!(format!("grant scan {owner}: {e}")));
                    break;
                }
            };
            let handle = head.get("handle").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let surface = head.get("surface").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let kind = head.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let est = head.get("est_minutes").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let bmonth = head
                .get("budget_month")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Blacksmith heads reserve at grant time (queued
            // claims never reserved at claim time).
            if kind == "blacksmith" && !bmonth.is_empty() && bmonth == month {
                let reserve = t.exec(
                    "UPDATE placement_budgets \
                    SET reserved_minutes = reserved_minutes + ?, updated_at = ? \
                    WHERE month = ? AND surface = ? \
                    AND used_minutes + reserved_minutes + ? <= budget_minutes",
                    &[
                        Arg::Float(est),
                        Arg::Text(now.to_string()),
                        Arg::Text(bmonth.clone()),
                        Arg::Text(surface.clone()),
                        Arg::Float(est),
                    ],
                );
                match reserve {
                    Ok(r) if r.affected == 0 => break,
                    Ok(_) => {}
                    Err(e) => {
                        errors.push(json!(format!("grant reserve {handle}: {e}")));
                        break;
                    }
                }
            }
            let grant_at = coord_parse_iso(now)
                .map(|base| coord_iso_from_epoch(base + GRANT_TTL_SECS))
                .unwrap_or_else(|| now.to_string());
            let step: Result<(), TErr> = (|| {
                t.exec(
                    "UPDATE placement_claims SET state = 'granted', expires_at = ?, \
                    updated_at = ? WHERE handle = ? AND state = 'queued'",
                    &[
                        Arg::Text(grant_at),
                        Arg::Text(now.to_string()),
                        Arg::Text(handle.clone()),
                    ],
                )?;
                t.exec(
                    "UPDATE placement_owner_state SET claims_active = claims_active + 1, \
                    updated_at = ? WHERE owner = ?",
                    &[Arg::Text(now.to_string()), Arg::Text(owner.clone())],
                )?;
                Ok(())
            })();
            match step {
                Ok(()) => granted += 1,
                Err(e) => {
                    errors.push(json!(format!("grant {handle}: {e}")));
                    break;
                }
            }
        }
    }
    json!({
        "ok": true,
        "refreshed": refreshed,
        "expired": expired,
        "settled": settled,
        "granted": granted,
        "errors": errors,
    })
}

// ---------------------------------------------------------------------------
// Proof-bar tests 1–3 (#13765 slice 1): claim gate, budget refuse,
// confirm/release — against a rusqlite-backed fake of the Turso
// pipeline endpoint (the lifecycle reprioritize proof-bar pattern —
// real SQL semantics, so the conditional-UPDATE slot gate and the
// budget reserve are exercised, not mimicked).
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod placement_coord_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    fn fake_backend() -> TursoBackend {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for ddl in SCHEMA {
            conn.execute_batch(ddl).unwrap();
        }
        let db = Arc::new(Mutex::new(conn));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let db_thread = db.clone();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let Ok(sock) = sock else { return };
                let db_conn = db_thread.clone();
                std::thread::spawn(move || handle_conn(sock, db_conn));
            }
        });
        TursoBackend::new(base, "test".to_string(), ureq::agent())
    }

    fn handle_conn(mut sock: std::net::TcpStream, db: Arc<Mutex<rusqlite::Connection>>) {
        let mut reader = BufReader::new(sock.try_clone().unwrap());
        loop {
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                return;
            }
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if let Some((name, value)) = trimmed.split_once(':') {
                    if name.trim().eq_ignore_ascii_case("content-length") {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0u8; content_length];
            if reader.read_exact(&mut body).is_err() {
                return;
            }
            let payload = run_pipeline(&db, &body);
            let text = payload.to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n{}",
                text.len(),
                text
            );
            if sock.write_all(resp.as_bytes()).is_err() {
                return;
            }
        }
    }

    fn run_pipeline(db: &Arc<Mutex<rusqlite::Connection>>, body: &[u8]) -> Value {
        let req: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let conn = db.lock().unwrap();
        let mut results = Vec::new();
        for r in req
            .get("requests")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
        {
            match r.get("type").and_then(|v| v.as_str()) {
                Some("close") => {
                    results.push(json!({"type": "ok", "response": {"type": "close"}}));
                }
                Some("execute") => {
                    let stmt = r.get("stmt").cloned().unwrap_or(Value::Null);
                    let sql = stmt.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                    let mut params: Vec<rusqlite::types::Value> = Vec::new();
                    if let Some(args) = stmt.get("args").and_then(|v| v.as_array()) {
                        for a in args {
                            params.push(match a.get("type").and_then(|v| v.as_str()) {
                                Some("text") => rusqlite::types::Value::Text(
                                    a.get("value")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                ),
                                Some("integer") => rusqlite::types::Value::Integer(
                                    a.get("value")
                                        .and_then(|v| v.as_str())
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(0),
                                ),
                                // hrana floats arrive as JSON numbers;
                                // stringified mirrors arrive as strings.
                                Some("float") => rusqlite::types::Value::Real(
                                    a.get("value").and_then(|v| {
                                        v.as_f64().or_else(|| {
                                            v.as_str()?.parse::<f64>().ok()
                                        })
                                    }).unwrap_or(0.0),
                                ),
                                _ => rusqlite::types::Value::Null,
                            });
                        }
                    }
                    results.push(execute_sql(&conn, sql, &params));
                }
                _ => {}
            }
        }
        json!({"results": results, "baton": null})
    }

    fn cell_json(v: rusqlite::types::ValueRef) -> Value {
        use rusqlite::types::ValueRef::*;
        match v {
            Null => json!({"type": "null"}),
            Integer(i) => json!({"type": "integer", "value": i.to_string()}),
            Real(f) => json!({"type": "float", "value": f}),
            Text(s) => json!({"type": "text", "value": String::from_utf8_lossy(s)}),
            Blob(b) => json!({"type": "blob", "value": b.iter().map(|x| format!("{x:02x}")).collect::<String>()}),
        }
    }

    fn execute_sql(
        conn: &rusqlite::Connection,
        sql: &str,
        params: &[rusqlite::types::Value],
    ) -> Value {
        let is_query = sql.trim_start().to_ascii_uppercase().starts_with("SELECT");
        if !is_query {
            let head = sql.trim_start().to_ascii_uppercase();
            if head.starts_with("BEGIN") || head.starts_with("COMMIT") || head.starts_with("ROLLBACK") {
                return match conn.execute_batch(sql) {
                    Ok(()) => json!({"type": "ok", "response": {"type": "execute", "result": {
                        "cols": [], "rows": [], "affected_row_count": 0, "last_insert_rowid": null}}}),
                    Err(e) => {
                        json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}})
                    }
                };
            }
        }
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
            }
        };
        if is_query {
            let cols: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
            let mut rows_out = Vec::new();
            match stmt.query(rusqlite::params_from_iter(params.iter())) {
                Ok(mut rows) => loop {
                    match rows.next() {
                        Ok(Some(row)) => {
                            let mut cells = Vec::new();
                            for i in 0..cols.len() {
                                cells.push(cell_json(row.get_ref(i).unwrap()));
                            }
                            rows_out.push(Value::Array(cells));
                        }
                        Ok(None) => break,
                        Err(e) => {
                            return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
                        }
                    }
                },
                Err(e) => {
                    return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
                }
            }
            json!({"type": "ok", "response": {"type": "execute", "result": {
                "cols": cols.iter().map(|c| json!({"name": c, "decltype": null})).collect::<Vec<_>>(),
                "rows": rows_out, "affected_row_count": 0, "last_insert_rowid": null}}})
        } else {
            match stmt.execute(rusqlite::params_from_iter(params.iter())) {
                Ok(changed) => json!({"type": "ok", "response": {"type": "execute", "result": {
                    "cols": [], "rows": [], "affected_row_count": changed,
                    "last_insert_rowid": conn.last_insert_rowid().to_string()}}}),
                Err(e) => {
                    json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}})
                }
            }
        }
    }

    fn claim(t: &TursoBackend, handle: &str, surface: &str, extra: Value) -> Value {
        let mut req = json!({"handle": handle, "surface": surface, "seat": "seat-1"});
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            req[k] = v;
        }
        verb_placement_claim(t, &req)
    }

    /// Test 1: the slot gate grants up to the registry cap, then
    /// queues. developing-today's cap is 20: 20 claims on
    /// fleet-org grant, the 21st waits with FIFO position 1, and
    /// the owner's load view shows occupancy 21.
    #[test]
    fn slot_gate_grants_to_cap_then_queues() {
        let t = fake_backend();
        for i in 0..20 {
            let out = claim(&t, &format!("h{i:02}"), "fleet-org", json!({}));
            assert_eq!(out["ok"], true, "{i}: {out}");
            assert_eq!(out["status"], "claimed", "{i}: {out}");
            assert_eq!(out["owner"], "developing-today", "{i}: {out}");
        }
        let out = claim(&t, "h20", "fleet-org", json!({}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["status"], "queued", "{out}");
        assert_eq!(out["position"], 1, "{out}");
        assert_eq!(out["owner_load"]["cap"], 20, "{out}");
        assert_eq!(out["owner_load"]["claims_active"], 20, "{out}");
        assert_eq!(out["owner_load"]["occupancy"], 20, "{out}");
        // A second waiter takes FIFO position 2.
        let out2 = claim(&t, "h21", "fleet-org", json!({}));
        assert_eq!(out2["status"], "queued", "{out2}");
        assert_eq!(out2["position"], 2, "{out2}");
        // The queued claim is visible by status from any seat.
        let st = verb_placement_claim_status(&t, &json!({"handle": "h20"}));
        assert_eq!(st["found"], true, "{st}");
        assert_eq!(st["claim"]["state"], "queued", "{st}");
    }

    /// Test 2: the Blacksmith budget refuses terminally once spent,
    /// and settling converts a reservation to use. Budget is 3000
    /// free min/mo: reserve 2000, a second 2000 refuses with no
    /// claim row; settling the first at 500 actual frees the rest
    /// so a 2500 estimate fits exactly (500 + 2500 <= 3000) while
    /// 2600 does not.
    #[test]
    fn budget_refuse_and_settle() {
        let t = fake_backend();
        let a = claim(
            &t,
            "bs-a",
            "fleet-org-blacksmith",
            json!({"est_minutes": 2000.0, "allow_paid": 0.0}),
        );
        assert_eq!(a["status"], "claimed", "{a}");
        let b = claim(
            &t,
            "bs-b",
            "fleet-org-blacksmith",
            json!({"est_minutes": 2000.0, "allow_paid": 0.0}),
        );
        assert_eq!(b["status"], "refused", "{b}");
        assert_eq!(b["reason"], "budget-exhausted", "{b}");
        let st = verb_placement_claim_status(&t, &json!({"handle": "bs-b"}));
        assert_eq!(st["found"], false, "{st}");
        // Confirm then settle at 500 actual minutes.
        let c = verb_placement_confirm(&t, &json!({"handle": "bs-a", "run_id": 7}));
        assert_eq!(c["confirmed"], true, "{c}");
        let r = verb_placement_release(
            &t,
            &json!({"handle": "bs-a", "actual_minutes": 500.0}),
        );
        assert_eq!(r["state"], "settled", "{r}");
        assert_eq!(r["actual_minutes"], 500.0, "{r}");
        // 500 used + 2500 estimate fits; 2600 does not.
        let d = claim(
            &t,
            "bs-d",
            "fleet-org-blacksmith",
            json!({"est_minutes": 2500.0, "allow_paid": 0.0}),
        );
        assert_eq!(d["status"], "claimed", "{d}");
        let e = claim(
            &t,
            "bs-e",
            "fleet-org-blacksmith",
            json!({"est_minutes": 2600.0, "allow_paid": 0.0}),
        );
        assert_eq!(e["status"], "refused", "{e}");
    }

    /// Test 3: confirm/release close the loop and free the slot.
    /// Unknown handles confirm nothing and resolve unfound;
    /// release is idempotent and the owner's counter returns to 0.
    #[test]
    fn confirm_release_lifecycle() {
        let t = fake_backend();
        let a = claim(&t, "lc-a", "fleet-org", json!({}));
        assert_eq!(a["status"], "claimed", "{a}");
        assert_eq!(a["owner_load"]["claims_active"], 1, "{a}");
        let st = verb_placement_claim_status(&t, &json!({"handle": "lc-a"}));
        assert_eq!(st["found"], true, "{st}");
        assert_eq!(st["claim"]["state"], "claimed", "{st}");
        // Unknown handles confirm nothing and resolve unfound.
        let miss = verb_placement_confirm(&t, &json!({"handle": "nope"}));
        assert_eq!(miss["confirmed"], false, "{miss}");
        let misst = verb_placement_claim_status(&t, &json!({"handle": "nope"}));
        assert_eq!(misst["found"], false, "{misst}");
        // Confirm carries the run id; a second confirm is a no-op.
        let c = verb_placement_confirm(&t, &json!({"handle": "lc-a", "run_id": 99}));
        assert_eq!(c["confirmed"], true, "{c}");
        assert_eq!(c["run_id"], 99, "{c}");
        let c2 = verb_placement_confirm(&t, &json!({"handle": "lc-a"}));
        assert_eq!(c2["confirmed"], false, "{c2}");
        // Release closes a confirmed non-Blacksmith row. The
        // owner's counter stays until the sweep recomputes it
        // from the claims table (its run may still be observed
        // live) — while releasing a still-held `claimed` slot
        // frees the counter immediately.
        let r = verb_placement_release(&t, &json!({"handle": "lc-a"}));
        assert_eq!(r["released"], true, "{r}");
        assert_eq!(r["state"], "released", "{r}");
        let r2 = verb_placement_release(&t, &json!({"handle": "lc-a"}));
        assert_eq!(r2["released"], true, "{r2}");
        let b = claim(&t, "lc-b", "fleet-org", json!({}));
        assert_eq!(b["status"], "claimed", "{b}");
        let rb = verb_placement_release(&t, &json!({"handle": "lc-b"}));
        assert_eq!(rb["state"], "released", "{rb}");
        let load = verb_placement_load(&t, &json!({}));
        let dt = load["owners"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["owner"] == "developing-today")
            .cloned()
            .unwrap();
        assert_eq!(dt["claims_active"], 1, "{load}");
        assert_eq!(dt["cap"], 20, "{load}");
        assert_eq!(load["owners"].as_array().unwrap().len(), 3, "{load}");
    }

    struct FakeGh {
        loads: HashMap<String, (i64, i64)>,
        live: HashMap<String, Vec<i64>>,
        terminal: HashMap<(String, i64), f64>,
    }

    impl CoordGh for FakeGh {
        fn load_counts(&self, repo: &str) -> Result<(i64, i64), String> {
            Ok(self.loads.get(repo).copied().unwrap_or((0, 0)))
        }
        fn live_run_ids(&self, repo: &str) -> Result<Vec<i64>, String> {
            Ok(self.live.get(repo).cloned().unwrap_or_default())
        }
        fn run_actual_minutes(&self, repo: &str, run_id: i64) -> Result<Option<f64>, String> {
            Ok(self.terminal.get(&(repo.to_string(), run_id)).copied())
        }
    }

    /// Test 4: saturation queues, and the sweep grants FIFO as
    /// slots free; a grant made on one seat resolves from another,
    /// and stale holds expire with their slots freed. developing-today
    /// fills to its cap of 20, two waiters queue, a release opens
    /// one slot, and the sweep grants exactly the older waiter.
    #[test]
    fn sweep_grants_fifo_and_expires_stale_holds() {
        let t = fake_backend();
        for i in 0..20 {
            let out = claim(&t, &format!("s{i:02}"), "fleet-org", json!({}));
            assert_eq!(out["status"], "claimed", "{i}: {out}");
        }
        let q1 = claim(&t, "sq-1", "fleet-org", json!({}));
        let q2 = claim(&t, "sq-2", "fleet-org", json!({}));
        assert_eq!(q1["status"], "queued", "{q1}");
        assert_eq!(q2["status"], "queued", "{q2}");
        // One slot frees.
        let r = verb_placement_release(&t, &json!({"handle": "s00"}));
        assert_eq!(r["state"], "released", "{r}");
        let gh = FakeGh {
            loads: HashMap::new(),
            live: HashMap::new(),
            terminal: HashMap::new(),
        };
        let now = now_iso();
        let sum = coordinator_sweep(&t, &gh, &now);
        assert_eq!(sum["ok"], true, "{sum}");
        assert_eq!(sum["refreshed"], 3, "{sum}");
        assert_eq!(sum["expired"], 0, "{sum}");
        assert_eq!(sum["settled"], 0, "{sum}");
        assert_eq!(sum["granted"], 1, "{sum}");
        assert!(sum["errors"].as_array().unwrap().is_empty(), "{sum}");
        // Exactly the older waiter holds the grant; the younger
        // still waits. The grant resolves cross-seat (the status
        // verb carries no seat — any seat reads the same row).
        let g1 = verb_placement_claim_status(&t, &json!({"handle": "sq-1"}));
        assert_eq!(g1["claim"]["state"], "granted", "{g1}");
        assert!(g1["claim"]["expires_at"].as_str().unwrap() > now.as_str(), "{g1}");
        let g2 = verb_placement_claim_status(&t, &json!({"handle": "sq-2"}));
        assert_eq!(g2["claim"]["state"], "queued", "{g2}");
        // A stale held slot (TTL lapsed) expires on the next pass
        // and its slot frees: free one more live slot, craft one
        // directly past its TTL, and the sweep expires the stale
        // hold and grants the remaining waiter.
        let r2 = verb_placement_release(&t, &json!({"handle": "s01"}));
        assert_eq!(r2["state"], "released", "{r2}");
        t.exec(
            "INSERT INTO placement_claims \
            (handle, owner, surface, kind, seat, state, est_minutes, \
            created_at, expires_at, updated_at) \
            VALUES ('stale-1', 'developing-today', 'fleet-org', 'github-actions', \
            'seat-9', 'claimed', 10, '2020-01-01T00:00:00Z', '2020-01-01T00:05:00Z', \
            '2020-01-01T00:00:00Z')",
            &[],
        )
        .unwrap();
        t.exec(
            "UPDATE placement_owner_state SET claims_active = claims_active + 1 \
            WHERE owner = 'developing-today'",
            &[],
        )
        .unwrap();
        let sum2 = coordinator_sweep(&t, &gh, &now_iso());
        assert_eq!(sum2["expired"], 1, "{sum2}");
        let st = verb_placement_claim_status(&t, &json!({"handle": "stale-1"}));
        assert_eq!(st["claim"]["state"], "expired", "{st}");
        // The freed slot went to the remaining waiter: sq-2 is now
        // granted and the owner is back at cap.
        let g2b = verb_placement_claim_status(&t, &json!({"handle": "sq-2"}));
        assert_eq!(g2b["claim"]["state"], "granted", "{g2b} {sum2}");
        assert_eq!(g2b["owner_load"]["claims_active"], 20, "{g2b} {sum2}");
    }
}
