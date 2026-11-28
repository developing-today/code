//! The local shortlist: a small file of ready work on this host.
//!
//! It is the third rung of the intake ladder. When no command was
//! pushed and the queue has nothing (or cannot be reached), an agent
//! takes the top entry here. The list is kept short on purpose: it is
//! what one host should do next, not a second backlog.
//!
//! **Refill by switching purpose.** A run that finds the list at or
//! under its low-water mark, with nobody already refilling, does not
//! take an entry. Its work for that run becomes filling the list
//! (`next` returns rung `refill`). The marker it leaves lasts
//! [`REFILL_FRESH_SECS`], so one run refills and the rest keep working.
//!
//! The file is JSON, written whole through a temporary file and a
//! rename, under a lock file, so two runs on one host never interleave.
//! A file that does not parse is an error naming the path: the list is
//! never silently reset.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{
    AgentError, DEFAULT_REPO, EXIT_NO_WORK, EXIT_OK, R, World, check_name, check_repo, parse, usage,
};

pub const DEFAULT_LOW_WATER: u64 = 3;
pub const DEFAULT_TARGET: u64 = 8;
/// How long one run's refill marker keeps other runs from refilling.
pub const REFILL_FRESH_SECS: u64 = 30 * 60;
/// How long a handed-out issue is kept from being handed out again here.
pub const HANDED_TTL_SECS: u64 = 2 * 60 * 60;
/// The rank of an entry no label ranked.
pub const RANK_UNRANKED: u64 = 6;

const LOCK_STALE_SECS: u64 = 30;
const LOCK_TRIES: u32 = 100;
const LOCK_PAUSE_MS: u64 = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub repo: String,
    pub issue: u64,
    pub title: String,
    /// Lower is sooner, like a queue band. See [`rank_of`].
    pub rank: u64,
    pub why: String,
    pub labels: Vec<String>,
    pub added_by: String,
    pub added_at: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Handed {
    pub repo: String,
    pub issue: u64,
    pub to: String,
    pub at: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shortlist {
    pub entries: Vec<Entry>,
    /// Who is refilling, and since when.
    pub refill: Option<(String, u64)>,
    /// When a fill last finished. A list that is still low just after
    /// a fill is low because little work is eligible, not because
    /// nobody filled it.
    pub filled_at: Option<u64>,
    pub handed: Vec<Handed>,
}

/// The rank a set of labels gives an issue. Lower is sooner.
pub fn rank_of(labels: &[String]) -> u64 {
    const ORDER: [&str; 6] = [
        "expedite",
        "priority/critical",
        "now",
        "priority/high",
        "next",
        "priority/normal",
    ];
    ORDER
        .iter()
        .position(|l| labels.iter().any(|have| have == l))
        .map(|p| p as u64)
        .unwrap_or(RANK_UNRANKED)
}

impl Entry {
    pub fn to_json(&self) -> Value {
        json!({
            "repo": self.repo,
            "issue": self.issue,
            "title": self.title,
            "rank": self.rank,
            "why": self.why,
            "labels": self.labels,
            "added_by": self.added_by,
            "added_at": self.added_at,
        })
    }
    fn from_json(v: &Value) -> Option<Entry> {
        Some(Entry {
            repo: v.get("repo")?.as_str()?.to_string(),
            issue: v.get("issue")?.as_u64()?,
            title: v.get("title")?.as_str()?.to_string(),
            rank: v
                .get("rank")
                .and_then(Value::as_u64)
                .unwrap_or(RANK_UNRANKED),
            why: v
                .get("why")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            labels: v
                .get("labels")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|l| l.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            added_by: v
                .get("added_by")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            added_at: v.get("added_at").and_then(Value::as_u64).unwrap_or(0),
        })
    }
}

impl Shortlist {
    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "entries": self.entries.iter().map(Entry::to_json).collect::<Vec<_>>(),
            "refill": self.refill.as_ref().map(|(by, at)| json!({"by": by, "at": at})),
            "filled_at": self.filled_at,
            "handed": self
                .handed
                .iter()
                .map(|h| json!({"repo": h.repo, "issue": h.issue, "to": h.to, "at": h.at}))
                .collect::<Vec<_>>(),
        })
    }

    fn from_json(v: &Value) -> Option<Shortlist> {
        if v.get("version").and_then(Value::as_u64) != Some(1) {
            return None;
        }
        let entries = v
            .get("entries")?
            .as_array()?
            .iter()
            .map(Entry::from_json)
            .collect::<Option<Vec<_>>>()?;
        let refill = match v.get("refill") {
            None | Some(Value::Null) => None,
            Some(r) => Some((r.get("by")?.as_str()?.to_string(), r.get("at")?.as_u64()?)),
        };
        let handed = match v.get("handed").and_then(Value::as_array) {
            None => Vec::new(),
            Some(a) => a
                .iter()
                .map(|h| {
                    Some(Handed {
                        repo: h.get("repo")?.as_str()?.to_string(),
                        issue: h.get("issue")?.as_u64()?,
                        to: h.get("to")?.as_str()?.to_string(),
                        at: h.get("at")?.as_u64()?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        };
        Some(Shortlist {
            entries,
            refill,
            filled_at: v.get("filled_at").and_then(Value::as_u64),
            handed,
        })
    }

    /// Drop what time has retired: old handed-out records and a refill
    /// marker past its window.
    pub fn prune(&mut self, now: u64) {
        self.handed
            .retain(|h| now.saturating_sub(h.at) < HANDED_TTL_SECS);
        if let Some((_, at)) = &self.refill {
            if now.saturating_sub(*at) >= REFILL_FRESH_SECS {
                self.refill = None;
            }
        }
    }

    pub fn has(&self, repo: &str, issue: u64) -> bool {
        self.entries
            .iter()
            .any(|e| e.repo == repo && e.issue == issue)
    }

    pub fn recently_handed(&self, repo: &str, issue: u64) -> bool {
        self.handed
            .iter()
            .any(|h| h.repo == repo && h.issue == issue)
    }

    /// Add an entry unless the issue is already listed or was handed
    /// out here within the window. True when it was added.
    pub fn add(&mut self, entry: Entry) -> bool {
        if self.has(&entry.repo, entry.issue) || self.recently_handed(&entry.repo, entry.issue) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// Hand out the soonest entry: lowest rank, then oldest. An
    /// entry handed out here within the window is never handed out
    /// again, even when a stale row for it is still listed: a
    /// priority reservation removes its row (see
    /// [`Shortlist::reserve_from`]), but a row written before that,
    /// or restored from an old file, must not resurrect the issue.
    pub fn take(&mut self, to: &str, now: u64) -> Option<Entry> {
        let index = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !self.recently_handed(&e.repo, e.issue))
            .min_by_key(|(_, e)| (e.rank, e.added_at, e.issue))
            .map(|(i, _)| i)?;
        let entry = self.entries.remove(index);
        self.handed.push(Handed {
            repo: entry.repo.clone(),
            issue: entry.issue,
            to: to.to_string(),
            at: now,
        });
        Some(entry)
    }

    /// Record a priority hand-out: the first of `found` not handed
    /// out here within the window. The hand-out is total: any
    /// shortlist row for the same repo/issue is removed in the same
    /// step, so a later [`Shortlist::take`] cannot hand the issue
    /// out a second time (#14651).
    pub fn reserve_from(&mut self, to: &str, now: u64, found: &[Entry]) -> Option<Entry> {
        let pick = found
            .iter()
            .find(|e| !self.recently_handed(&e.repo, e.issue))
            .cloned()?;
        self.entries
            .retain(|e| !(e.repo == pick.repo && e.issue == pick.issue));
        self.handed.push(Handed {
            repo: pick.repo.clone(),
            issue: pick.issue,
            to: to.to_string(),
            at: now,
        });
        Some(pick)
    }

    pub fn is_low(&self, low_water: u64) -> bool {
        (self.entries.len() as u64) <= low_water
    }

    /// Whether a fill finished within the refill window.
    pub fn filled_recently(&self, now: u64) -> bool {
        self.filled_at
            .is_some_and(|at| now.saturating_sub(at) < REFILL_FRESH_SECS)
    }

    /// Whether this run should switch purpose and refill: the list is
    /// low, nobody has claimed the refill, and no fill just finished.
    /// The last part is what lets a host with only one or two eligible
    /// issues work them, in place of refilling for ever.
    pub fn needs_refill(&self, low_water: u64, now: u64) -> bool {
        self.is_low(low_water) && self.refill.is_none() && !self.filled_recently(now)
    }
}

// ---- the file ----

/// `--shortlist`, else `$FLEET_SHORTLIST`, else the XDG state path.
pub fn path(world: &dyn World, flag: Option<String>) -> R<PathBuf> {
    if let Some(p) = flag.or_else(|| world.env("FLEET_SHORTLIST")) {
        return Ok(PathBuf::from(p));
    }
    if let Some(state) = world.env("XDG_STATE_HOME") {
        return Ok(PathBuf::from(state).join("fleet").join("shortlist.json"));
    }
    world
        .home()
        .map(|h| h.join(".local/state/fleet/shortlist.json"))
        .ok_or_else(|| {
            AgentError::Down(
                "no place for the shortlist: set --shortlist, FLEET_SHORTLIST or HOME".to_string(),
            )
        })
}

pub fn load(path: &Path) -> R<Shortlist> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Shortlist::default()),
        Err(e) => {
            return Err(AgentError::Down(format!(
                "cannot read the shortlist {}: {e}",
                path.display()
            )));
        }
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .as_ref()
        .and_then(Shortlist::from_json)
        .ok_or_else(|| {
            AgentError::Down(format!(
                "the shortlist {} is not a version-1 shortlist file; move it aside to start a new one",
                path.display()
            ))
        })
}

fn save(path: &Path, list: &Shortlist) -> R<()> {
    let down = |what: &str, e: std::io::Error| {
        AgentError::Down(format!(
            "cannot {what} the shortlist {}: {e}",
            path.display()
        ))
    };
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|e| down("make the directory for", e))?;
        }
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let text = serde_json::to_string_pretty(&list.to_json()).unwrap_or_default();
    let mut file = fs::File::create(&tmp).map_err(|e| down("write", e))?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|e| down("write", e))?;
    fs::rename(&tmp, path).map_err(|e| down("replace", e))
}

struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn lock(path: &Path) -> R<Lock> {
    let lock_path = path.with_extension("lock");
    if let Some(dir) = lock_path.parent() {
        if !dir.as_os_str().is_empty() {
            let _ = fs::create_dir_all(dir);
        }
    }
    for _ in 0..LOCK_TRIES {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => return Ok(Lock(lock_path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // A lock older than any honest hold belongs to a run that died.
                let stale = fs::metadata(&lock_path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age.as_secs() >= LOCK_STALE_SECS);
                if stale {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(LOCK_PAUSE_MS));
            }
            Err(e) => {
                return Err(AgentError::Down(format!(
                    "cannot lock the shortlist {}: {e}",
                    path.display()
                )));
            }
        }
    }
    Err(AgentError::Down(format!(
        "the shortlist {} stayed locked by another run",
        path.display()
    )))
}

/// Read, change and write the list as one step. The closure's answer
/// is returned; the file is rewritten only when the list changed.
pub fn update<T>(path: &Path, now: u64, change: impl FnOnce(&mut Shortlist) -> T) -> R<T> {
    let _held = lock(path)?;
    let before = load(path)?;
    let mut list = before.clone();
    list.prune(now);
    let answer = change(&mut list);
    if list != before {
        save(path, &list)?;
    }
    Ok(answer)
}

/// Pick the first of `found` that this host has not handed out
/// lately, and record the hand-out, in one locked update. Choosing
/// outside the lock would let two runs at the same moment both pick
/// the same issue.
pub fn reserve(path: &Path, now: u64, to: &str, found: &[Entry]) -> R<Option<Entry>> {
    update(path, now, |list| list.reserve_from(to, now, found))
}

// ---- the verbs ----

fn status_json(path: &Path, list: &Shortlist, low_water: u64) -> Value {
    json!({
        "path": path.display().to_string(),
        "count": list.entries.len(),
        "low_water": low_water,
        "low": list.is_low(low_water),
        "refill": list.refill.as_ref().map(|(by, at)| json!({"by": by, "at": at})),
        "entries": list.entries.iter().map(Entry::to_json).collect::<Vec<_>>(),
    })
}

/// `fleet agent shortlist <verb>`.
pub fn run(world: &dyn World, args: &[String]) -> R<(Value, i32)> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(usage(
            "shortlist needs a verb: status, add, take, fill or clear",
        ));
    };
    let now = world.now();
    match verb.as_str() {
        "status" => {
            let a = parse(rest, &[], &["--shortlist", "--low-water"])?;
            a.no_positional("shortlist status")?;
            let file = path(world, a.last("--shortlist"))?;
            let low_water = a.uint("--low-water")?.unwrap_or(DEFAULT_LOW_WATER);
            let mut list = load(&file)?;
            list.prune(now);
            Ok((status_json(&file, &list, low_water), EXIT_OK))
        }
        "add" => {
            let a = parse(
                rest,
                &[],
                &[
                    "--shortlist",
                    "--repo",
                    "--issue",
                    "--title",
                    "--rank",
                    "--why",
                    "--agent",
                ],
            )?;
            a.no_positional("shortlist add")?;
            let file = path(world, a.last("--shortlist"))?;
            let issue = a
                .uint("--issue")?
                .filter(|n| *n > 0)
                .ok_or_else(|| usage("shortlist add needs --issue <number>"))?;
            let title = a
                .last("--title")
                .ok_or_else(|| usage("shortlist add needs --title"))?;
            let repo = check_repo(&a.last("--repo").unwrap_or_else(|| DEFAULT_REPO.to_string()))?;
            let by = match a.last("--agent").or_else(|| world.env("FLEET_AGENT")) {
                Some(n) => check_name("--agent", &n)?,
                None => String::new(),
            };
            let entry = Entry {
                repo,
                issue,
                title,
                rank: a.uint("--rank")?.unwrap_or(RANK_UNRANKED),
                why: a.last("--why").unwrap_or_default(),
                labels: Vec::new(),
                added_by: by,
                added_at: now,
            };
            let shown = entry.to_json();
            let (added, count) = update(&file, now, |list| {
                let added = list.add(entry);
                (added, list.entries.len())
            })?;
            Ok((
                json!({"added": added, "count": count, "entry": shown,
                       "note": if added { "" } else { "already listed, or handed out here in the last two hours" }}),
                EXIT_OK,
            ))
        }
        "take" => {
            let a = parse(rest, &[], &["--shortlist", "--agent"])?;
            a.no_positional("shortlist take")?;
            let file = path(world, a.last("--shortlist"))?;
            let to = match a.last("--agent").or_else(|| world.env("FLEET_AGENT")) {
                Some(n) => check_name("--agent", &n)?,
                None => String::new(),
            };
            let (taken, left) = update(&file, now, |list| {
                let taken = list.take(&to, now);
                (taken, list.entries.len())
            })?;
            match taken {
                Some(e) => Ok((json!({"entry": e.to_json(), "left": left}), EXIT_OK)),
                None => Ok((json!({"entry": null, "left": 0}), EXIT_NO_WORK)),
            }
        }
        "fill" => {
            let a = parse(rest, &[], &["--shortlist", "--repo", "--target", "--agent"])?;
            a.no_positional("shortlist fill")?;
            let file = path(world, a.last("--shortlist"))?;
            let repo = check_repo(&a.last("--repo").unwrap_or_else(|| DEFAULT_REPO.to_string()))?;
            let target = a.uint("--target")?.unwrap_or(DEFAULT_TARGET).clamp(1, 50);
            let by = match a.last("--agent").or_else(|| world.env("FLEET_AGENT")) {
                Some(n) => check_name("--agent", &n)?,
                None => String::new(),
            };
            let found = crate::next::candidates(world, &repo, target as usize * 2)
                .map_err(AgentError::Down)?;
            let seen = found.len();
            let (added, count) = update(&file, now, |list| {
                let mut added = 0u64;
                for mut entry in found {
                    if (list.entries.len() as u64) >= target {
                        break;
                    }
                    entry.added_by = by.clone();
                    entry.added_at = now;
                    if list.add(entry) {
                        added += 1;
                    }
                }
                // The fill is done: whoever held the purpose is released,
                // and the time is kept so a list that stays low is
                // worked, not refilled again at once.
                list.refill = None;
                list.filled_at = Some(now);
                (added, list.entries.len())
            })?;
            Ok((
                json!({"added": added, "count": count, "target": target, "candidates_seen": seen}),
                EXIT_OK,
            ))
        }
        "clear" => {
            let a = parse(rest, &[], &["--shortlist"])?;
            a.no_positional("shortlist clear")?;
            let file = path(world, a.last("--shortlist"))?;
            let removed = update(&file, now, |list| {
                let n = list.entries.len();
                list.entries.clear();
                list.refill = None;
                n
            })?;
            Ok((json!({"removed": removed}), EXIT_OK))
        }
        other => Err(usage(format!(
            "unknown shortlist verb '{other}': status, add, take, fill or clear"
        ))),
    }
}
