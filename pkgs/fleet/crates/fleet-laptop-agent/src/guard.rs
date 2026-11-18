//! Primary-checkout guards (code#10095). A primary checkout is a
//! directory whose `.git` is a directory; a linked worktree's `.git` is
//! a file. Two refusals, both overridable with `LAPTOP_AGENT_PRIMARY_OK=1`:
//!
//! * a job whose working directory is a primary checkout;
//! * a git mutation (commit, push, reset, ...) whose target is one.
//!
//! The checks run on the path as this client sees it. The daemon's
//! working directory is not visible here, so a job with no `--cwd` is
//! not checked.

use std::path::Path;

pub const PRIMARY_OK_ENV: &str = "LAPTOP_AGENT_PRIMARY_OK";

/// Git subcommands that change a repository.
const MUTATING: &[&str] = &[
    "add",
    "am",
    "apply",
    "branch",
    "checkout",
    "cherry-pick",
    "clean",
    "commit",
    "gc",
    "merge",
    "mv",
    "pull",
    "push",
    "rebase",
    "reset",
    "restore",
    "revert",
    "rm",
    "stash",
    "switch",
    "tag",
    "update-ref",
    "worktree",
];

pub fn is_primary_checkout(dir: &Path) -> bool {
    dir.join(".git").is_dir()
}

/// Refuse a job that would run inside a primary checkout, or a git
/// mutation aimed at one. `words` is the argv, or the shell string split
/// on whitespace for `--shell`.
pub fn check_job(cwd: Option<&str>, words: &[String]) -> Result<(), String> {
    if std::env::var(PRIMARY_OK_ENV).as_deref() == Ok("1") {
        return Ok(());
    }
    if let Some(dir) = cwd
        && is_primary_checkout(Path::new(dir))
    {
        return Err(format!(
            "refusing to run a job in primary checkout {dir} (use a worktree, or {PRIMARY_OK_ENV}=1)"
        ));
    }
    if let Some((sub, target)) = git_mutation(words) {
        let base = target.or(cwd);
        if let Some(dir) = base
            && is_primary_checkout(Path::new(dir))
        {
            return Err(format!(
                "refusing git {sub} against primary checkout {dir} (use a worktree, or {PRIMARY_OK_ENV}=1)"
            ));
        }
    }
    Ok(())
}

/// The mutating subcommand and the `-C` target, when `words` is a git
/// command that changes a repository.
fn git_mutation(words: &[String]) -> Option<(String, Option<&str>)> {
    if words.first().map(String::as_str) != Some("git") {
        return None;
    }
    let mut target = None;
    let mut i = 1;
    while i < words.len() {
        let w = words[i].as_str();
        if w == "-C" {
            target = words.get(i + 1).map(String::as_str);
            i += 2;
        } else if w.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    let sub = words.get(i)?;
    if MUTATING.contains(&sub.as_str()) {
        Some((sub.clone(), target))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn git_mutations_are_found_past_global_options() {
        assert_eq!(
            git_mutation(&words("git -C /x commit -m m")),
            Some(("commit".into(), Some("/x")))
        );
        assert_eq!(git_mutation(&words("git status")), None);
        assert_eq!(git_mutation(&words("ls -la")), None);
    }

    #[test]
    fn a_worktree_is_not_a_primary_checkout() {
        let dir = std::env::temp_dir().join(format!("la-guard-{}", std::process::id()));
        let primary = dir.join("primary");
        let worktree = dir.join("worktree");
        std::fs::create_dir_all(primary.join(".git")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: /elsewhere").unwrap();
        assert!(is_primary_checkout(&primary));
        assert!(!is_primary_checkout(&worktree));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
