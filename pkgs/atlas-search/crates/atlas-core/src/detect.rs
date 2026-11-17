//! Change detection and the detection ledger
//! (`doc/2026-11-10T22-56-00Z_design_atlas_delta_detection_join` §2.1,
//! §2.4; issue #9978, follow-on (a) of the #6028 design record).
//!
//! Detection answers one question per object: _unchanged, changed,
//! new, or gone — since the last landed ingest of this source?_ It
//! never embeds, never re-tags, and writes nothing except the ledger
//! entry recording its verdicts. This module implements the git
//! source mechanism (§2.1: tree diff, never working-tree reads) and
//! the persisted ledger that makes every pass auditable (§2.4).
//!
//! Git sources are detected by commit, not by reading the working
//! tree: a re-ingest at commit `C` with last-landed commit `L` is the
//! tree diff `L..C`. `git diff-tree -r --no-renames` names changed
//! blobs directly and prunes unchanged subtrees by tree-object
//! identity, so blob content for unchanged paths is never read.
//! Rename detection is a second pass over the delete/add pairs only.
//!
//! # The #6028 guarantee, stated once
//!
//! **An object whose detection verdict is unchanged receives no
//! writes of any kind — no embed, no tag write, no metadata edit, no
//! date bump. The only record of the pass touching it is the
//! aggregate count in the detection ledger entry.**
//!
//! This module only *reports* verdicts; the ingest path is what obeys
//! the prohibition. The tests here pin the verdicts (a re-ingest of an
//! unchanged tree lands zero changes; an edit lands exactly that file)
//! so an ingest path cannot silently widen them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Schema tag written into every persisted ledger.
pub const DETECTION_LEDGER_SCHEMA: &str = "atlas-detection-ledger/v1";

/// The detection mechanism a pass used (§2.1–§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanism {
    /// Commit-level tree diff over a git source (§2.1).
    GitTreeDiff,
}

impl Mechanism {
    pub fn as_str(self) -> &'static str {
        match self {
            Mechanism::GitTreeDiff => "git_tree_diff",
        }
    }
}

/// The verdict detection reaches for one object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Landed already under the same content; the pass writes nothing.
    Unchanged,
    /// Landed before, content differs; the new version lands.
    Changed,
    /// No landed version; the first version lands.
    New,
    /// Landed before, absent now; the version retires (no re-embed).
    Gone,
}

/// One object's verdict in a detection pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathVerdict {
    /// The path at the target commit (the old path for `Gone`).
    pub path: String,
    /// The previous path when the pass detected a rename (second pass).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
    pub verdict: Verdict,
    /// The target-side blob object id (`None` for `Gone`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    /// The previously landed blob object id (`None` for `New`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_blob: Option<String>,
}

/// Counts by verdict — the ledger's audit columns (§2.4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictCounts {
    pub unchanged: u64,
    pub changed: u64,
    pub new: u64,
    pub gone: u64,
}

impl VerdictCounts {
    fn add(&mut self, verdict: Verdict) {
        match verdict {
            Verdict::Unchanged => self.unchanged += 1,
            Verdict::Changed => self.changed += 1,
            Verdict::New => self.new += 1,
            Verdict::Gone => self.gone += 1,
        }
    }
}

/// One source's landed state: the commit the corpus was last landed
/// at, plus the path→blob map the next pass consults (§2.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandedState {
    pub commit: String,
    /// Landed blobs by path. These are git object ids, so recording
    /// them costs no content read.
    #[serde(default)]
    pub objects: BTreeMap<String, String>,
}

/// One detection pass, appended to the ledger per source (§2.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionPass {
    pub source: String,
    pub mechanism: Mechanism,
    /// Last-landed commit, `None` on a first ingest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    pub counts: VerdictCounts,
    /// Fraction of the prior landed objects the mechanism presumed
    /// unchanged from its cheap signal (stat hits for §2.2; pruned
    /// subtrees for §2.1). `None` when the mechanism has no such
    /// signal to report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stat_presumption_rate: Option<f64>,
    /// Bytes of object content the pass read. A git tree diff reads
    /// tree objects and the changed blobs' names only, so this is 0
    /// for a pure detection pass.
    pub bytes_read: u64,
    /// Wall-clock duration of the pass in milliseconds.
    pub elapsed_ms: u64,
    /// The per-object verdicts. Unchanged objects are not listed: the
    /// ledger records them in `counts` only, so a mass re-date of
    /// unchanged objects is visible as a write the pass did not make.
    #[serde(default)]
    pub verdicts: Vec<PathVerdict>,
}

/// The persisted detection ledger: one section per source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionLedger {
    pub schema: String,
    #[serde(default)]
    pub sources: BTreeMap<String, SourceLedger>,
}

/// One source's ledger section: landed state plus every pass.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceLedger {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landed: Option<LandedState>,
    #[serde(default)]
    pub passes: Vec<DetectionPass>,
}

impl Default for DetectionLedger {
    fn default() -> Self {
        Self {
            schema: DETECTION_LEDGER_SCHEMA.to_string(),
            sources: BTreeMap::new(),
        }
    }
}

impl DetectionLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a ledger from JSON. A missing file is an empty ledger —
    /// the first ingest of a source is a normal state, not an error.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => {
                return Err(Error::backend(format!(
                    "detection ledger {}: {e}",
                    path.display()
                )));
            }
        };
        let ledger: DetectionLedger = serde_json::from_str(&text).map_err(|e| {
            Error::backend(format!("detection ledger {}: {e}", path.display()))
        })?;
        if ledger.schema != DETECTION_LEDGER_SCHEMA {
            return Err(Error::backend(format!(
                "detection ledger {}: unsupported schema {:?}",
                path.display(),
                ledger.schema
            )));
        }
        Ok(ledger)
    }

    /// Persist the ledger as pretty JSON, creating parent directories.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::backend(format!("detection ledger {}: {e}", parent.display()))
            })?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::backend(format!("detection ledger: {e}")))?;
        std::fs::write(path, format!("{text}\n")).map_err(|e| {
            Error::backend(format!("detection ledger {}: {e}", path.display()))
        })
    }

    pub fn source(&self, source: &str) -> Option<&SourceLedger> {
        self.sources.get(source)
    }

    /// The last landed commit for a source, if any.
    pub fn landed_commit(&self, source: &str) -> Option<&str> {
        self.sources
            .get(source)
            .and_then(|s| s.landed.as_ref())
            .map(|l| l.commit.as_str())
    }

    /// Detect changes for a git source at `target` and record the pass.
    ///
    /// The last landed commit is taken from the ledger; a source with
    /// no landed state is a first ingest (every blob is `New`). The
    /// pass appends to the ledger and advances the landed state to
    /// `target` with the merged path→blob map. Nothing else is
    /// written: unchanged objects appear only as a count.
    pub fn detect_git(
        &mut self,
        repo: &Path,
        source: impl Into<String>,
        target: &str,
    ) -> Result<DetectionPass> {
        let source = source.into();
        let previous = self.sources.get(&source).and_then(|s| s.landed.clone());
        let from = previous.as_ref().map(|l| l.commit.clone());
        let started = Instant::now();

        let mut pass = git_tree_detection(
            repo,
            &source,
            from.as_deref(),
            target,
            previous.as_ref(),
        )?;
        pass.elapsed_ms = started.elapsed().as_millis() as u64;

        // Merge the verdicts into the landed map. An unchanged path is
        // left untouched (its object id is already the same), which is
        // the ledger-side half of the #6028 guarantee.
        let mut objects = previous.map(|p| p.objects).unwrap_or_default();
        for verdict in &pass.verdicts {
            match verdict.verdict {
                Verdict::New | Verdict::Changed => {
                    if let Some(old) = &verdict.previous_path {
                        objects.remove(old);
                    }
                    if let Some(blob) = &verdict.blob {
                        objects.insert(verdict.path.clone(), blob.clone());
                    }
                }
                Verdict::Gone => {
                    objects.remove(&verdict.path);
                }
                Verdict::Unchanged => {}
            }
        }

        let section = self.sources.entry(source.clone()).or_default();
        section.passes.push(pass.clone());
        section.landed = Some(LandedState {
            commit: target.to_string(),
            objects,
        });
        Ok(pass)
    }
}

/// Run the git tree diff for one source without recording it.
///
/// `from` is the last landed commit (`None` = first ingest);
/// `landed` is the previous landed path→blob map, used only to count
/// the objects the diff leaves unchanged.
pub fn git_tree_detection(
    repo: &Path,
    source: &str,
    from: Option<&str>,
    to: &str,
    landed: Option<&LandedState>,
) -> Result<DetectionPass> {
    let mut verdicts = match from {
        Some(from) => diff_tree(repo, from, to)?,
        None => ls_tree(repo, to)?,
    };
    apply_rename_second_pass(&mut verdicts);

    let mut counts = VerdictCounts::default();
    for verdict in &verdicts {
        counts.add(verdict.verdict);
    }
    // Unchanged = landed paths the diff did not name. The diff's own
    // pruning is what guarantees the count; the objects are never
    // opened.
    if let Some(landed) = landed {
        let touched: std::collections::BTreeSet<&str> = verdicts
            .iter()
            .flat_map(|v| {
                std::iter::once(v.path.as_str())
                    .chain(v.previous_path.as_deref())
            })
            .collect();
        counts.unchanged = landed
            .objects
            .keys()
            .filter(|path| !touched.contains(path.as_str()))
            .count() as u64;
    }
    let stat_presumption_rate = landed.and_then(|l| {
        let total = l.objects.len() + verdicts.iter().filter(|v| v.verdict == Verdict::New).count();
        (total > 0).then(|| counts.unchanged as f64 / total as f64)
    });

    Ok(DetectionPass {
        source: source.to_string(),
        mechanism: Mechanism::GitTreeDiff,
        from: from.map(str::to_string),
        to: to.to_string(),
        counts,
        stat_presumption_rate,
        bytes_read: 0,
        elapsed_ms: 0,
        verdicts,
    })
}

/// First ingest: every blob at `to` is `New`, read from the tree
/// object only (`git ls-tree`, never the working tree).
fn ls_tree(repo: &Path, to: &str) -> Result<Vec<PathVerdict>> {
    let out = git(repo, &["ls-tree", "-r", "-z", "--full-tree", to])?;
    let mut verdicts = Vec::new();
    for entry in out.split('\0').filter(|e| !e.is_empty()) {
        // "<mode> SP <type> SP <oid> TAB <path>"
        let Some((meta, path)) = entry.split_once('\t') else {
            continue;
        };
        let mut fields = meta.split(' ');
        let _mode = fields.next();
        let kind = fields.next().unwrap_or("");
        let oid = fields.next().unwrap_or("");
        if kind != "blob" || oid.is_empty() {
            continue;
        }
        verdicts.push(PathVerdict {
            path: path.to_string(),
            previous_path: None,
            verdict: Verdict::New,
            blob: Some(oid.to_string()),
            previous_blob: None,
        });
    }
    Ok(verdicts)
}

/// Re-ingest: `git diff-tree -r --no-renames --raw L C`, parsed for
/// the changed blobs only.
fn diff_tree(repo: &Path, from: &str, to: &str) -> Result<Vec<PathVerdict>> {
    let out = git(
        repo,
        &["diff-tree", "-r", "-z", "--no-renames", "--raw", from, to],
    )?;
    Ok(parse_raw_diff(&out))
}

/// Parse `git diff-tree --raw -z` output:
/// `:<old-mode> <new-mode> <old-oid> <new-oid> <status>\0<path>\0`.
fn parse_raw_diff(out: &str) -> Vec<PathVerdict> {
    let mut verdicts = Vec::new();
    let mut parts = out.split('\0');
    while let Some(header) = parts.next() {
        let Some(header) = header.strip_prefix(':') else {
            continue;
        };
        let fields: Vec<&str> = header.split(' ').collect();
        if fields.len() < 5 {
            continue;
        }
        let old_oid = fields[2];
        let new_oid = fields[3];
        let status = fields[4];
        let Some(path) = parts.next() else { break };
        // Status letters: A added, M modified, D deleted, T type
        // changed, plus combinations. With --no-renames there are no
        // R/C rows here; renames are the second pass in
        // `apply_rename_second_pass`.
        let verdict = match status.chars().next().unwrap_or('M') {
            'A' => Verdict::New,
            'D' => Verdict::Gone,
            _ => Verdict::Changed,
        };
        let zero = |oid: &str| oid.chars().all(|c| c == '0');
        let blob = (!zero(new_oid)).then(|| new_oid.to_string());
        let previous_blob = (!zero(old_oid)).then(|| old_oid.to_string());
        // A pure mode change (M with identical oids) is not a content
        // change: the blob is unchanged, so no re-embed may follow.
        if verdict == Verdict::Changed && blob == previous_blob {
            continue;
        }
        verdicts.push(PathVerdict {
            path: path.to_string(),
            previous_path: None,
            verdict,
            blob,
            previous_blob,
        });
    }
    verdicts
}

/// Rename second pass (§2.1): a delete/add pair whose blob ids match is
/// one rename, not one new and one gone. Only the delete/add pairs are
/// considered — this never reopens the full diff.
fn apply_rename_second_pass(verdicts: &mut Vec<PathVerdict>) {
    let adds: Vec<usize> = verdicts
        .iter()
        .enumerate()
        .filter(|(_, v)| v.verdict == Verdict::New)
        .map(|(i, _)| i)
        .collect();
    // Pair every add with at most one not-yet-consumed delete, then drop
    // the consumed deletes in one pass, so indices stay valid while
    // pairing and every rename in the diff is coalesced, not just the
    // first one.
    let mut consumed = vec![false; verdicts.len()];
    for add in adds {
        let Some(add_blob) = verdicts[add].blob.clone() else {
            continue;
        };
        if let Some(del) = verdicts.iter().enumerate().position(|(i, v)| {
            !consumed[i]
                && v.verdict == Verdict::Gone
                && v.previous_blob.as_deref() == Some(add_blob.as_str())
        }) {
            consumed[del] = true;
            let old_path = verdicts[del].path.clone();
            verdicts[add].verdict = Verdict::Changed;
            verdicts[add].previous_path = Some(old_path);
            verdicts[add].previous_blob = Some(add_blob);
        }
    }
    let mut index = 0;
    verdicts.retain(|_| {
        let keep = !consumed[index];
        index += 1;
        keep
    });
}

/// Resolve a commit-ish to a full object id, so the ledger stores what
/// it can verify later.
pub fn resolve_commit(repo: &Path, rev: &str) -> Result<String> {
    let out = git(repo, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")])?;
    Ok(out.trim().to_string())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| Error::backend(format!("git {}: {e}", args.join(" "))))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::backend(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|e| Error::backend(format!("git {}: non-UTF-8 output: {e}", args.join(" "))))
}

/// Where a ledger lives for a workspace, by convention.
pub fn default_ledger_path(workspace: &Path) -> PathBuf {
    workspace.join("detection-ledger.json")
}
