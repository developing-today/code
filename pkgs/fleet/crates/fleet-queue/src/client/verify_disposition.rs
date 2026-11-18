//! `repo-queue verify-disposition` (#10611): the readback a terminal
//! completion must carry. Ported onto the split client (#8588).

use serde_json::{Map, Value, json};

use super::*;

fn gh_api(gh: &GhRunner, path: String) -> Result<Value, String> {
    gh.json(&["api".to_string(), path])
}

/// `owner/name` and number of a follow-up issue given as a number
/// (`9400`, `#9400`) or a GitHub issue URL. Only issues in the same
/// repository are accepted: their `Refs #n` links are only
/// meaningful there.
fn follow_up_issue_number(raw: &str, repo: &str) -> Result<i64, String> {
    let t = raw.trim();
    if let Ok(n) = t.trim_start_matches('#').parse::<i64>() {
        return Ok(n);
    }
    let marker = format!("github.com/{repo}/issues/");
    match t.find(&marker) {
        Some(idx) => {
            let digits: String = t[idx + marker.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            digits
                .parse::<i64>()
                .map_err(|_| format!("argument --follow-up-issue: no issue number in {raw:?}"))
        }
        None => Err(format!(
            "argument --follow-up-issue: {raw:?} is not an issue number or an issue URL in {repo}"
        )),
    }
}

/// `verify-disposition` (#10611): build the verified-disposition record
/// a terminal completion must carry, by reading GitHub back now — never
/// from a remembered claim. Prints the record `complete --result` wants
/// (`--json` for the raw object, a summary otherwise). Exits 1 when the
/// record would not pass the broker's gate, so a lane cannot paste a
/// record it did not check. A follow-up task is checked by the broker
/// at completion (only the queue knows whether it is live).
pub(super) fn cmd_verify_disposition(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "verify-disposition",
        rest,
        &[
            "repo",
            "pr",
            "follow-up-task",
            "follow-up-issue",
            "follow-up-pr",
            "reason",
            "staleness-seconds",
        ],
        &["json"],
    );
    reject_extra_pos("verify-disposition", &args, 1);
    let issue = args.pos_i64(0, "issue");
    let repo = args.get("repo").unwrap_or_else(|| {
        die(
            2,
            "verify-disposition: --repo owner/name is required (the readback \
             is only meaningful against a named repository)",
        )
    });
    let staleness = args
        .get_i64("staleness-seconds")
        .unwrap_or(crate::disposition::DEFAULT_STALENESS_SECONDS);
    if staleness <= 0 {
        die(2, "argument --staleness-seconds: must be positive");
    }

    // `--pr 12,34` — the parser rejects duplicate flags, so the linked
    // set is one comma-separated value (#10611 provenance column).
    let mut linked: Vec<i64> = Vec::new();
    if let Some(raw) = args.get("pr") {
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let n = part
                .trim_start_matches('#')
                .parse::<i64>()
                .unwrap_or_else(|_| die(2, &format!("argument --pr: invalid PR number {part:?}")));
            if !linked.contains(&n) {
                linked.push(n);
            }
        }
    }

    let gh = GhRunner::system();
    // The REST issue carries `closed_by`, which the broker needs to
    // tell an authorized close from an automated one.
    let issue_json = gh_api(&gh, format!("repos/{repo}/issues/{issue}")).unwrap_or_else(|e| {
        die(
            1,
            &format!("verify-disposition: cannot read issue #{issue} in {repo}: {e}"),
        )
    });
    let prs_json = gh
        .json(&[
            "pr".to_string(),
            "list".to_string(),
            "--repo".to_string(),
            repo.clone(),
            "--state".to_string(),
            "all".to_string(),
            "--limit".to_string(),
            "300".to_string(),
            "--json".to_string(),
            "number,state,isDraft,mergedAt,body".to_string(),
        ])
        .unwrap_or_else(|e| {
            die(
                1,
                &format!("verify-disposition: cannot list PRs in {repo}: {e}"),
            )
        });
    // The 300 newest PRs are not the whole history: #9245 — a draft
    // that still carries the #9155 work — sits below that window. Ask
    // search for bodies naming the issue (a failed search is not
    // evidence of absence, so it never aborts the readback), union by
    // number, then fetch any explicitly linked PR the union lacks.
    let mut prs: Vec<Value> = prs_json.as_array().cloned().unwrap_or_default();
    let has = |prs: &[Value], n: i64| {
        prs.iter()
            .any(|p| p.get("number").and_then(|x| x.as_i64()) == Some(n))
    };
    if let Ok(found) = gh.json(&[
        "pr".to_string(),
        "list".to_string(),
        "--repo".to_string(),
        repo.clone(),
        "--state".to_string(),
        "all".to_string(),
        "--search".to_string(),
        format!("{issue} in:body"),
        "--limit".to_string(),
        "100".to_string(),
        "--json".to_string(),
        "number,state,isDraft,mergedAt,body".to_string(),
    ]) {
        for item in found.as_array().cloned().unwrap_or_default() {
            if let Some(n) = item.get("number").and_then(|n| n.as_i64()) {
                if !has(&prs, n) {
                    prs.push(item);
                }
            }
        }
    }
    let successor = args.get("follow-up-pr").map(|raw| {
        raw.trim()
            .trim_start_matches('#')
            .parse::<i64>()
            .unwrap_or_else(|_| {
                die(
                    2,
                    &format!("argument --follow-up-pr: invalid PR number {raw:?}"),
                )
            })
    });
    // A named successor must be read back like any linked PR.
    let mut wanted = linked.clone();
    if let Some(n) = successor {
        if !wanted.contains(&n) {
            wanted.push(n);
        }
    }
    for n in &wanted {
        if has(&prs, *n) {
            continue;
        }
        match gh.json(&[
            "pr".to_string(),
            "view".to_string(),
            n.to_string(),
            "--repo".to_string(),
            repo.clone(),
            "--json".to_string(),
            "number,state,isDraft,mergedAt,body".to_string(),
        ]) {
            Ok(item) => prs.push(item),
            Err(e) => die(
                1,
                &format!("verify-disposition: cannot read linked PR #{n} in {repo}: {e}"),
            ),
        }
    }
    let prs_json = Value::Array(prs);

    let reason = args.get("reason");
    let any_target = args.get("follow-up-task").is_some()
        || args.get("follow-up-issue").is_some()
        || successor.is_some();
    if any_target && reason.is_none() {
        die(
            2,
            "verify-disposition: a follow-up needs --reason (why the remainder \
             belongs there)",
        );
    }
    let mut follow_up = Map::new();
    if let Some(raw) = args.get("follow-up-issue") {
        let n = follow_up_issue_number(&raw, &repo).unwrap_or_else(|e| die(2, &e));
        let readback = gh_api(&gh, format!("repos/{repo}/issues/{n}")).unwrap_or_else(|e| {
            die(
                1,
                &format!("verify-disposition: cannot read follow-up issue #{n} in {repo}: {e}"),
            )
        });
        let url = format!("https://github.com/{repo}/issues/{n}");
        if let Value::Object(map) =
            crate::disposition::follow_up_issue(&url, &readback, reason.as_deref().unwrap_or(""))
        {
            follow_up.extend(map);
        }
    }
    if let Some(task) = args.get("follow-up-task") {
        match task.parse::<i64>() {
            Ok(n) => {
                follow_up.insert("task_id".to_string(), json!(n));
            }
            Err(_) => die(
                2,
                &format!("argument --follow-up-task: invalid int value {task:?}"),
            ),
        }
    }
    if let Some(n) = successor {
        follow_up.insert("successor_pr".to_string(), json!(n));
    }
    if let Some(reason) = reason {
        if any_target {
            follow_up.insert("reason".to_string(), json!(reason));
        }
    }
    let record = crate::disposition::from_gh(
        &issue_json,
        &prs_json,
        &linked,
        if follow_up.is_empty() {
            None
        } else {
            Some(Value::Object(follow_up))
        },
        &utcnow_iso(),
    );

    let checked = crate::disposition::parse_record(&record).and_then(|rec| {
        crate::disposition::check(
            &rec,
            Some(issue),
            &linked,
            now_micros() / 1_000_000,
            staleness,
        )
    });

    if args.has("json") {
        print_json(&record);
    } else {
        let verdict = s(&record, "verdict");
        let pulls = record
            .get("pulls")
            .and_then(|p| p.as_array())
            .map(|p| {
                p.iter()
                    .map(|x| {
                        format!(
                            "#{} {} {}",
                            x.get("number").and_then(|n| n.as_i64()).unwrap_or(0),
                            s(x, "state"),
                            if x.get("is_draft").and_then(|d| d.as_bool()).unwrap_or(false) {
                                "draft"
                            } else {
                                ""
                            }
                        )
                        .trim_end()
                        .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!(
            "issue #{issue} in {repo}: {}",
            record["issue"]["state"].as_str().unwrap_or("?")
        );
        println!("verdict: {verdict}");
        println!(
            "pulls: {}",
            if pulls.is_empty() {
                "none".to_string()
            } else {
                pulls
            }
        );
        println!("checked_at: {}", s(&record, "checked_at"));
        match &checked {
            Ok(()) => println!("gate: would pass (a follow-up task is checked by the broker)"),
            Err(e) => println!("gate: would REJECT — {e}"),
        }
    }
    match checked {
        Ok(()) => 0,
        Err(e) => {
            eprint(&format!("verify-disposition: {e}"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::follow_up_issue_number;

    #[test]
    fn follow_up_issue_is_a_number_or_a_same_repo_url() {
        assert_eq!(follow_up_issue_number("9400", "o/r"), Ok(9400));
        assert_eq!(follow_up_issue_number("#9400", "o/r"), Ok(9400));
        assert_eq!(
            follow_up_issue_number("https://github.com/o/r/issues/9400", "o/r"),
            Ok(9400)
        );
        assert!(follow_up_issue_number("https://github.com/x/y/issues/9400", "o/r").is_err());
    }
}
