//! Queue/GitHub sync verbs for `fleet gh` (developing-today/code#10973,
//! step 2.2 of #9016).
//!
//! The recurring scripted flows — pm-stack dispatch's gh fallback,
//! the opencode claimant's label reads and agent notes, launch_one's
//! hard-stop check and launch notes, and the job-body flows named by
//! #9016 (program tending, stewardship verdict posting, pool
//! reviews) — used to compose raw `gh issue` / `gh pr` argument
//! lists at every call site. The `sync-*` shadow verbs give those
//! callers one typed surface instead: each verb declares its whole
//! command line (the #3352 operator-script contract), validates it
//! strictly, and rewrites to the canonical gh call. The rewrite is
//! applied in `gh::run` before the close guard, the closure layer
//! and the metadata create path see the arguments, so a sync call
//! inherits the token rotation, the retry-safety rule, value-free
//! errors and the audit trail exactly as a hand-written passthrough
//! call does — and a caller's stdout contract is gh's own output,
//! byte-exact, because the final attempt's output is never wrapped.
//!
//! Every verb takes `--repo OWNER/REPO`. Item targets are an
//! `issue`/`pr` kind plus a number where the underlying call is
//! kinded. The verbs are:
//!
//!   sync-labels       <issue|pr> <n>           label names, JSON list
//!   sync-view         <issue|pr> <n>           labels,state,title JSON
//!   sync-comment      <issue|pr> <n> --body T | --body-file P
//!   sync-issue-create --title T (--body T | --body-file P) [--label L]...
//!   sync-issue-get    <n>                      one issue, API JSON
//!   sync-labels-add   <n> --label L [--label L]...
//!   sync-label-ensure --label L --color C --description D
//!   sync-issue-close  <n> --reason <fleet closure reason>
//!
//! `sync-issue-close` deliberately speaks the fleet closure taxonomy
//! (#9221, gh_closure::REASONS), not gh's `--reason` state-reason
//! spellings: the canonical call it rewrites to is the closure
//! layer's own input shape, so the close carries its reason comment
//! and closure record like every other fleet close.
//!
//! `--body-file` is read by this layer and substituted as `--body`,
//! so the canonical call is identical whichever form the caller
//! used; an unreadable file refuses the verb before any GitHub
//! write. Errors name the verb, the flag and the stage only — never
//! a body, a token, or any other value the caller passed.

/// The sync verb names, in help order. Real `gh` has none of these,
/// so intercepting them cannot shadow a passthrough call.
pub const VERBS: [&str; 8] = [
    "sync-labels",
    "sync-view",
    "sync-comment",
    "sync-issue-create",
    "sync-issue-get",
    "sync-labels-add",
    "sync-label-ensure",
    "sync-issue-close",
];

fn usage(verb: &str) -> &'static str {
    match verb {
        "sync-labels" => "fleet gh sync-labels --repo OWNER/REPO <issue|pr> <number>",
        "sync-view" => "fleet gh sync-view --repo OWNER/REPO <issue|pr> <number>",
        "sync-comment" => {
            "fleet gh sync-comment --repo OWNER/REPO <issue|pr> <number> --body TEXT | --body-file PATH"
        }
        "sync-issue-create" => {
            "fleet gh sync-issue-create --repo OWNER/REPO --title TITLE (--body TEXT | --body-file PATH) [--label LABEL]..."
        }
        "sync-issue-get" => "fleet gh sync-issue-get --repo OWNER/REPO <number>",
        "sync-labels-add" => {
            "fleet gh sync-labels-add --repo OWNER/REPO <number> --label LABEL [--label LABEL]..."
        }
        "sync-label-ensure" => {
            "fleet gh sync-label-ensure --repo OWNER/REPO --label LABEL --color COLOR --description TEXT"
        }
        "sync-issue-close" => {
            "fleet gh sync-issue-close --repo OWNER/REPO <number> --reason REASON"
        }
        _ => "fleet gh sync-*",
    }
}

fn err(verb: &str, message: &str) -> String {
    format!("fleet gh {verb}: {message}\nusage: {}", usage(verb))
}

/// One verb's strictly-parsed command line: declared value flags
/// (repeatable flags collect), plus the positional arguments. Any
/// token the verb did not declare is a usage error (#3352).
struct Parsed {
    flags: Vec<(String, String)>,
    positionals: Vec<String>,
}

impl Parsed {
    fn get(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn get_all(&self, name: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn require(&self, verb: &str, name: &str) -> Result<String, String> {
        self.get(name)
            .map(str::to_string)
            .ok_or_else(|| err(verb, &format!("missing required flag '{name}'")))
    }
}

fn parse_declared(
    verb: &str,
    tail: &[String],
    value_flags: &[&str],
) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        flags: Vec::new(),
        positionals: Vec::new(),
    };
    let mut i = 0;
    while i < tail.len() {
        let a = tail[i].as_str();
        if let Some(long) = a.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let full = format!("--{name}");
            if !value_flags.contains(&full.as_str()) {
                return Err(err(verb, &format!("unexpected argument '{a}'")));
            }
            let value = match inline {
                Some(v) => v,
                None => {
                    i += 1;
                    match tail.get(i) {
                        Some(v) => v.clone(),
                        None => {
                            return Err(err(verb, &format!("flag '{full}' needs a value")));
                        }
                    }
                }
            };
            parsed.flags.push((full, value));
        } else if a.starts_with('-') && a.len() > 1 {
            return Err(err(verb, &format!("unexpected argument '{a}'")));
        } else {
            parsed.positionals.push(a.to_string());
        }
        i += 1;
    }
    Ok(parsed)
}

/// `--repo` is always OWNER/REPO: two non-empty segments of the
/// characters GitHub allows in owner and repository names.
fn repo(parsed: &Parsed, verb: &str) -> Result<String, String> {
    let raw = parsed.require(verb, "--repo")?;
    let mut parts = raw.split('/');
    let ok = match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) => {
            !owner.is_empty()
                && !name.is_empty()
                && owner
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        }
        _ => false,
    };
    if ok {
        Ok(raw)
    } else {
        Err(err(
            verb,
            &format!("--repo '{raw}' is not an OWNER/REPO slug"),
        ))
    }
}

/// An item number: digits, with one optional leading `#`.
fn item_number(verb: &str, raw: &str) -> Result<String, String> {
    let digits = raw.strip_prefix('#').unwrap_or(raw);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        Ok(digits.to_string())
    } else {
        Err(err(verb, &format!("'{raw}' is not an item number")))
    }
}

fn kind(verb: &str, raw: &str) -> Result<String, String> {
    match raw {
        "issue" | "pr" => Ok(raw.to_string()),
        _ => Err(err(
            verb,
            &format!("'{raw}' is not an item kind: use 'issue' or 'pr'"),
        )),
    }
}

/// The `<issue|pr> <number>` positional pair most verbs take.
fn kinded_item(verb: &str, parsed: &Parsed) -> Result<(String, String), String> {
    if parsed.positionals.len() != 2 {
        return Err(err(
            verb,
            "expected exactly two arguments: <issue|pr> <number>",
        ));
    }
    Ok((
        kind(verb, &parsed.positionals[0])?,
        item_number(verb, &parsed.positionals[1])?,
    ))
}

/// The bare `<number>` positional the issue-only verbs take.
fn lone_item(verb: &str, parsed: &Parsed) -> Result<String, String> {
    if parsed.positionals.len() != 1 {
        return Err(err(verb, "expected exactly one argument: <number>"));
    }
    item_number(verb, &parsed.positionals[0])
}

fn no_positionals(verb: &str, parsed: &Parsed) -> Result<(), String> {
    match parsed.positionals.first() {
        Some(extra) => Err(err(verb, &format!("unexpected argument '{extra}'"))),
        None => Ok(()),
    }
}

/// Exactly one of `--body` / `--body-file`; a file is read here so
/// the canonical call carries `--body` either way, and an unreadable
/// file refuses the verb before any GitHub write.
fn body(verb: &str, parsed: &Parsed) -> Result<String, String> {
    match (parsed.get("--body"), parsed.get("--body-file")) {
        (Some(_), Some(_)) => Err(err(
            verb,
            "give exactly one of '--body' and '--body-file'",
        )),
        (Some(text), None) => Ok(text.to_string()),
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| {
            err(
                verb,
                &format!("cannot read --body-file '{path}': {e}"),
            )
        }),
        (None, None) => Err(err(
            verb,
            "missing required flag '--body' (or '--body-file')",
        )),
    }
}

/// Recognise a sync verb and rewrite it to the canonical gh call.
/// Returns Ok(None) for every other invocation, which then passes
/// through untouched. The rewrite is a pure argument transform —
/// the caller in `gh::run` feeds the result through the close
/// guard, the closure layer, the metadata create path and the
/// token rotation unchanged.
pub fn parse(args: &[String]) -> Result<Option<Vec<String>>, String> {
    let verb = match args.first().map(|s| s.as_str()) {
        Some(v) if VERBS.contains(&v) => v,
        _ => return Ok(None),
    };
    let tail = &args[1..];
    let canonical = match verb {
        "sync-labels" => {
            let p = parse_declared(verb, tail, &["--repo"])?;
            let repo = repo(&p, verb)?;
            let (kind, item) = kinded_item(verb, &p)?;
            vec![
                kind,
                "view".into(),
                item,
                "--repo".into(),
                repo,
                "--json".into(),
                "labels".into(),
                "--jq".into(),
                "[.labels[].name]".into(),
            ]
        }
        "sync-view" => {
            let p = parse_declared(verb, tail, &["--repo"])?;
            let repo = repo(&p, verb)?;
            let (kind, item) = kinded_item(verb, &p)?;
            vec![
                kind,
                "view".into(),
                item,
                "--repo".into(),
                repo,
                "--json".into(),
                "labels,state,title".into(),
            ]
        }
        "sync-comment" => {
            let p = parse_declared(verb, tail, &["--repo", "--body", "--body-file"])?;
            let repo = repo(&p, verb)?;
            let (kind, item) = kinded_item(verb, &p)?;
            let body = body(verb, &p)?;
            vec![
                kind,
                "comment".into(),
                item,
                "--repo".into(),
                repo,
                "--body".into(),
                body,
            ]
        }
        "sync-issue-create" => {
            let p = parse_declared(
                verb,
                tail,
                &["--repo", "--title", "--body", "--body-file", "--label"],
            )?;
            no_positionals(verb, &p)?;
            let repo = repo(&p, verb)?;
            let title = p.require(verb, "--title")?;
            let body = body(verb, &p)?;
            let mut out = vec![
                "issue".into(),
                "create".into(),
                "--repo".into(),
                repo,
                "--title".into(),
                title,
                "--body".into(),
                body,
            ];
            for label in p.get_all("--label") {
                out.push("--label".into());
                out.push(label.to_string());
            }
            out
        }
        "sync-issue-get" => {
            let p = parse_declared(verb, tail, &["--repo"])?;
            let repo = repo(&p, verb)?;
            let item = lone_item(verb, &p)?;
            vec!["api".into(), format!("repos/{repo}/issues/{item}")]
        }
        "sync-labels-add" => {
            let p = parse_declared(verb, tail, &["--repo", "--label"])?;
            let repo = repo(&p, verb)?;
            let item = lone_item(verb, &p)?;
            let labels = p.get_all("--label");
            if labels.is_empty() {
                return Err(err(verb, "missing required flag '--label'"));
            }
            let mut out = vec![
                "issue".into(),
                "edit".into(),
                item,
                "--repo".into(),
                repo,
            ];
            for label in labels {
                out.push("--add-label".into());
                out.push(label.to_string());
            }
            out
        }
        "sync-label-ensure" => {
            let p = parse_declared(
                verb,
                tail,
                &["--repo", "--label", "--color", "--description"],
            )?;
            no_positionals(verb, &p)?;
            let repo = repo(&p, verb)?;
            let label = p.require(verb, "--label")?;
            let color = p.require(verb, "--color")?;
            let description = p.require(verb, "--description")?;
            vec![
                "label".into(),
                "create".into(),
                label,
                "--repo".into(),
                repo,
                "--color".into(),
                color,
                "--description".into(),
                description,
                "--force".into(),
            ]
        }
        "sync-issue-close" => {
            let p = parse_declared(verb, tail, &["--repo", "--reason"])?;
            let repo = repo(&p, verb)?;
            let item = lone_item(verb, &p)?;
            let reason = p.require(verb, "--reason")?;
            if !crate::gh_closure::REASONS.contains(&reason.as_str()) {
                return Err(err(
                    verb,
                    &format!(
                        "unknown reason '{reason}': use one of {}",
                        crate::gh_closure::REASONS.join(", ")
                    ),
                ));
            }
            vec![
                "issue".into(),
                "close".into(),
                item,
                "--repo".into(),
                repo,
                "--reason".into(),
                reason,
            ]
        }
        _ => unreachable!("verb is drawn from VERBS"),
    };
    Ok(Some(canonical))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn rewrite(parts: &[&str]) -> Vec<String> {
        parse(&args(parts))
            .expect("parse should succeed")
            .expect("a sync verb should rewrite")
    }

    fn refused(parts: &[&str]) -> String {
        parse(&args(parts)).expect_err("parse should refuse")
    }

    #[test]
    fn non_sync_calls_pass_through_untouched() {
        assert_eq!(parse(&args(&["issue", "view", "1"])).unwrap(), None);
        assert_eq!(parse(&args(&["sync"])).unwrap(), None);
        assert_eq!(parse(&args(&[])).unwrap(), None);
        assert_eq!(parse(&args(&["sync-label"])).unwrap(), None);
    }

    #[test]
    fn sync_labels_rewrites_to_the_jq_label_read() {
        assert_eq!(
            rewrite(&[
                "sync-labels",
                "--repo",
                "developing-today/code",
                "issue",
                "10973"
            ]),
            args(&[
                "issue",
                "view",
                "10973",
                "--repo",
                "developing-today/code",
                "--json",
                "labels",
                "--jq",
                "[.labels[].name]"
            ])
        );
    }

    #[test]
    fn sync_view_rewrites_to_the_state_read() {
        assert_eq!(
            rewrite(&["sync-view", "--repo=developing-today/code", "pr", "#45"]),
            args(&[
                "pr",
                "view",
                "45",
                "--repo",
                "developing-today/code",
                "--json",
                "labels,state,title"
            ])
        );
    }

    #[test]
    fn sync_comment_rewrites_for_issues_and_prs() {
        assert_eq!(
            rewrite(&[
                "sync-comment",
                "--repo",
                "developing-today/code",
                "issue",
                "7",
                "--body",
                "note text"
            ]),
            args(&[
                "issue",
                "comment",
                "7",
                "--repo",
                "developing-today/code",
                "--body",
                "note text"
            ])
        );
        assert_eq!(
            rewrite(&[
                "sync-comment",
                "--repo",
                "developing-today/code",
                "pr",
                "9",
                "--body",
                "verdict"
            ]),
            args(&[
                "pr",
                "comment",
                "9",
                "--repo",
                "developing-today/code",
                "--body",
                "verdict"
            ])
        );
    }

    #[test]
    fn sync_comment_reads_a_body_file() {
        let path = std::env::temp_dir().join(format!(
            "fleet-gh-sync-test-{}-body.md",
            std::process::id()
        ));
        std::fs::write(&path, "from a file\n").unwrap();
        let out = rewrite(&[
            "sync-comment",
            "--repo",
            "developing-today/code",
            "issue",
            "7",
            "--body-file",
            path.to_str().unwrap(),
        ]);
        assert_eq!(out.last().map(|s| s.as_str()), Some("from a file\n"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn sync_comment_refuses_two_body_sources_and_a_missing_file() {
        let both = refused(&[
            "sync-comment",
            "--repo",
            "developing-today/code",
            "issue",
            "7",
            "--body",
            "x",
            "--body-file",
            "y",
        ]);
        assert!(both.contains("exactly one of '--body' and '--body-file'"), "{both}");
        let missing = refused(&[
            "sync-comment",
            "--repo",
            "developing-today/code",
            "issue",
            "7",
            "--body-file",
            "/nonexistent/fleet-gh-sync-body.md",
        ]);
        assert!(missing.contains("cannot read --body-file"), "{missing}");
        let neither = refused(&[
            "sync-comment",
            "--repo",
            "developing-today/code",
            "issue",
            "7",
        ]);
        assert!(neither.contains("missing required flag '--body'"), "{neither}");
    }

    #[test]
    fn sync_issue_create_rewrites_with_labels_in_order() {
        assert_eq!(
            rewrite(&[
                "sync-issue-create",
                "--repo",
                "developing-today/code",
                "--title",
                "tracking",
                "--body",
                "body",
                "--label",
                "queue/queued",
                "--label",
                "source/dispatch"
            ]),
            args(&[
                "issue",
                "create",
                "--repo",
                "developing-today/code",
                "--title",
                "tracking",
                "--body",
                "body",
                "--label",
                "queue/queued",
                "--label",
                "source/dispatch"
            ])
        );
    }

    #[test]
    fn sync_issue_get_rewrites_to_the_api_read() {
        assert_eq!(
            rewrite(&["sync-issue-get", "--repo", "developing-today/code", "3007"]),
            args(&["api", "repos/developing-today/code/issues/3007"])
        );
    }

    #[test]
    fn sync_labels_add_rewrites_and_requires_a_label() {
        assert_eq!(
            rewrite(&[
                "sync-labels-add",
                "--repo",
                "developing-today/code",
                "12",
                "--label",
                "queue/claimed",
                "--label",
                "work/assigned"
            ]),
            args(&[
                "issue",
                "edit",
                "12",
                "--repo",
                "developing-today/code",
                "--add-label",
                "queue/claimed",
                "--add-label",
                "work/assigned"
            ])
        );
        let none = refused(&["sync-labels-add", "--repo", "developing-today/code", "12"]);
        assert!(none.contains("missing required flag '--label'"), "{none}");
    }

    #[test]
    fn sync_label_ensure_rewrites_to_a_forced_create() {
        assert_eq!(
            rewrite(&[
                "sync-label-ensure",
                "--repo",
                "developing-today/code",
                "--label",
                "queue/queued",
                "--color",
                "7057ff",
                "--description",
                "Has a queued queue task"
            ]),
            args(&[
                "label",
                "create",
                "queue/queued",
                "--repo",
                "developing-today/code",
                "--color",
                "7057ff",
                "--description",
                "Has a queued queue task",
                "--force"
            ])
        );
    }

    #[test]
    fn sync_issue_close_speaks_the_closure_taxonomy() {
        assert_eq!(
            rewrite(&[
                "sync-issue-close",
                "--repo",
                "developing-today/code",
                "12",
                "--reason",
                "cancelled"
            ]),
            args(&[
                "issue",
                "close",
                "12",
                "--repo",
                "developing-today/code",
                "--reason",
                "cancelled"
            ])
        );
        // gh's own state-reason spelling is not the fleet taxonomy:
        // the closure layer would refuse it, so the verb refuses it
        // first, naming the accepted reasons.
        let gh_shape = refused(&[
            "sync-issue-close",
            "--repo",
            "developing-today/code",
            "12",
            "--reason",
            "not planned",
        ]);
        assert!(gh_shape.contains("unknown reason 'not planned'"), "{gh_shape}");
    }

    #[test]
    fn strict_parsing_refuses_undeclared_shapes() {
        // Unknown flag.
        assert!(refused(&[
            "sync-labels",
            "--repo",
            "developing-today/code",
            "issue",
            "1",
            "--json",
            "labels"
        ])
        .contains("unexpected argument '--json'"));
        // Missing --repo.
        assert!(refused(&["sync-labels", "issue", "1"]).contains("missing required flag '--repo'"));
        // Repo that is not an OWNER/REPO slug.
        assert!(refused(&["sync-labels", "--repo", "code", "issue", "1"])
            .contains("is not an OWNER/REPO slug"));
        assert!(refused(&["sync-labels", "--repo", "a/b/c", "issue", "1"])
            .contains("is not an OWNER/REPO slug"));
        // Kind that is neither issue nor pr.
        assert!(refused(&["sync-labels", "--repo", "a/b", "issuee", "1"])
            .contains("not an item kind"));
        // Item that is not a number.
        assert!(refused(&["sync-labels", "--repo", "a/b", "issue", "12a"])
            .contains("is not an item number"));
        // A value flag with no value.
        assert!(refused(&["sync-labels", "--repo"]).contains("flag '--repo' needs a value"));
        // A stray positional on a flag-only verb.
        assert!(refused(&[
            "sync-label-ensure",
            "--repo",
            "a/b",
            "--label",
            "l",
            "--color",
            "c",
            "--description",
            "d",
            "extra"
        ])
        .contains("unexpected argument 'extra'"));
    }
}
