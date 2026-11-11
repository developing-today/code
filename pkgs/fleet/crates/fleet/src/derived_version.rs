//! Derived per-app versioning for the fleet apps (issue #8566;
//! policy ruled on issue #5790, 2026-11-06).
//!
//! The version a fleet binary reports is DERIVED at build time from
//! the app's latest tag plus the commits since it — never a
//! hand-maintained literal. The grammar is the interim policy of
//! #5790 §1, with the #8566 acceptance forms:
//!
//!   * exactly on tag `<app>/vX.Y.Z`        -> `X.Y.Z`
//!   * on `main`, N commits after that tag  -> `X.Y.(Z+1)-dev.N+g<shorthash>`
//!   * on a feature branch, N commits after -> the same, plus
//!     `+<branch-slug>` build metadata
//!   * no `<app>/v*` tag yet (pre-first-tag) -> the `0.1.0-dev`
//!     fallback family: `0.1.0-dev.N+g<shorthash>` when the commit
//!     count and hash are known, bare `0.1.0-dev` when they are not
//!     (a build with no git history and no version passed in).
//!
//! Tag prefixes are the app's SHORT name for every app (#5790
//! refinement, 2026-11-06): one naming policy, no per-app mixing.
//! App names used in tags and deploys must be unique across apps;
//! if a short name would collide, the app name is fixed, not the
//! policy. Versions stay in 0.x until a separate 1.0 decision:
//! minor = features, patch = fixes, no major bumps.
//!
//! This module is pure (no I/O) so the build script can include it
//! by path and feed it `git describe` output, and the binary can
//! unit-test the same code the build script ran. The oracle for
//! every test below is the #5790 §1 policy text quoted above.

/// A released version from an `<app>/vX.Y.Z` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl ReleaseVersion {
    /// Parse the `X.Y.Z` part of a tag (no `v` prefix, no app part).
    pub fn parse(text: &str) -> Option<ReleaseVersion> {
        let mut parts = text.split('.');
        let major = parse_segment(parts.next()?)?;
        let minor = parse_segment(parts.next()?)?;
        let patch = parse_segment(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(ReleaseVersion {
            major,
            minor,
            patch,
        })
    }
}

impl std::fmt::Display for ReleaseVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_segment(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // No leading zeros: semver numeric segments forbid them, and a
    // `v01.2.3` tag is a typo to reject, not a version to honour.
    if text.len() > 1 && text.starts_with('0') {
        return None;
    }
    text.parse().ok()
}

/// Which kind of ref the build was cut from (#5790 §1 rule 1's
/// branch classes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchClass {
    /// HEAD is exactly the tagged commit: a release build.
    Tag,
    /// A build from `main` (or a detached HEAD with no branch):
    /// a prerelease, never a release.
    Main,
    /// A feature-branch build; carries the sanitized branch slug
    /// as extra build metadata.
    Feature(String),
}

/// Parse a per-app tag of the form `<app>/vX.Y.Z` for `app`.
/// Tags for any other app, and malformed tags, return `None` —
/// another app's tag must never version this app.
pub fn parse_app_tag(app: &str, tag: &str) -> Option<ReleaseVersion> {
    let version = tag.strip_prefix(app)?.strip_prefix("/v")?;
    ReleaseVersion::parse(version)
}

/// The parsed result of `git describe --tags --long --match '<app>/v*'`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Describe {
    pub tag: ReleaseVersion,
    /// Commits on HEAD since the tag (0 = exactly on the tag).
    pub distance: u64,
    /// Short commit hash, without git describe's `g` prefix.
    pub hash: Option<String>,
}

/// Parse `git describe` output for `app`. Accepts both the `--long`
/// form (`fleet/v0.4.2-7-g1a2b3c4`) and the exact-tag form
/// (`fleet/v0.4.2`, distance 0, no hash).
pub fn parse_describe(app: &str, output: &str) -> Option<Describe> {
    let text = output.trim();
    if let Some(tag) = parse_app_tag(app, text) {
        return Some(Describe {
            tag,
            distance: 0,
            hash: None,
        });
    }
    // Long form: strip the trailing `-<distance>-g<hash>` first,
    // because the tag itself contains no `-` but the split must
    // anchor at the END, not at the first dash in the string.
    let (head, hash) = text.rsplit_once("-g")?;
    if hash.is_empty() || !hash.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let (tag_text, distance_text) = head.rsplit_once('-')?;
    let tag = parse_app_tag(app, tag_text)?;
    let distance: u64 = distance_text.parse().ok()?;
    Some(Describe {
        tag,
        distance,
        hash: Some(hash.to_string()),
    })
}

/// Sanitize a branch name into semver build-metadata slug form:
/// lowercase alphanumerics and `-`, runs of anything else collapsed
/// to one `-`, no leading/trailing `-`, capped at 32 chars.
pub fn slugify_branch(branch: &str) -> String {
    let mut slug = String::new();
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.extend(ch.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(32);
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "feature".to_string()
    } else {
        slug
    }
}

/// Classify a branch name for [`derive`]. `on_tag` (a describe
/// distance of 0) wins over the branch name; an absent branch
/// (detached HEAD, the usual CI/deploy shape) off a tag is `Main`
/// class — deployed from main, a prerelease.
pub fn branch_class(branch: Option<&str>, on_tag: bool) -> BranchClass {
    if on_tag {
        return BranchClass::Tag;
    }
    match branch {
        None | Some("") | Some("main") | Some("master") => BranchClass::Main,
        Some(other) => BranchClass::Feature(slugify_branch(other)),
    }
}

/// The fallback base before an app's first tag: every app starts
/// at 0.1.0 (#5790 ruling 3).
pub const FIRST_VERSION: ReleaseVersion = ReleaseVersion {
    major: 0,
    minor: 1,
    patch: 0,
};

/// Derive the version string for one build.
///
/// * `tag` — the app's nearest tag version, if any tag exists yet.
/// * `distance` — commits since that tag; with no tag, the commit
///   count the caller can prove (the build script passes the
///   repo-wide count, so the number still grows monotonically).
/// * `hash` — the build commit's short hash, if known. It rides as
///   `+g<hash>` build metadata, which is what guarantees no two
///   distinct commits ever share a version string (#5790 §1).
/// * `branch` — the branch class of the build.
pub fn derive(
    tag: Option<ReleaseVersion>,
    distance: u64,
    hash: Option<&str>,
    branch: &BranchClass,
) -> String {
    let hash_part = hash
        .filter(|h| !h.is_empty())
        .map(|h| format!("+g{h}"))
        .unwrap_or_default();
    let slug_part = match branch {
        BranchClass::Feature(slug) => format!("+{slug}"),
        _ => String::new(),
    };
    match tag {
        Some(version) if distance == 0 => format!("{version}"),
        Some(version) => {
            // Past a tag on a non-release build: the NEXT patch
            // release's prerelease form, so the string sorts after
            // the tag it descends from and before its own release
            // (raw describe output would sort before the tag).
            let next = ReleaseVersion {
                patch: version.patch + 1,
                ..version
            };
            format!("{next}-dev.{distance}{hash_part}{slug_part}")
        }
        None => {
            // Pre-first-tag fallback (#8566 acceptance): the
            // 0.1.0-dev form. Bare `0.1.0-dev` only when neither a
            // distance nor a hash is known (history-less build).
            if distance == 0 && hash_part.is_empty() {
                format!("{FIRST_VERSION}-dev{slug_part}")
            } else {
                format!("{FIRST_VERSION}-dev.{distance}{hash_part}{slug_part}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "fleet";

    fn v(major: u64, minor: u64, patch: u64) -> ReleaseVersion {
        ReleaseVersion {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn exact_tag_is_the_clean_release() {
        // Oracle: #5790 §1 — exactly on the tag -> clean X.Y.Z.
        assert_eq!(
            derive(Some(v(0, 4, 2)), 0, Some("1a2b3c4"), &BranchClass::Tag),
            "0.4.2"
        );
    }

    #[test]
    fn main_after_tag_is_next_patch_dev() {
        // Oracle: #5790 §1 — on main, N commits after X.Y.Z ->
        // X.Y.(Z+1)-dev.N+g<shorthash> (its worked example shape).
        assert_eq!(
            derive(Some(v(0, 4, 2)), 7, Some("1a2b3c4"), &BranchClass::Main),
            "0.4.3-dev.7+g1a2b3c4"
        );
    }

    #[test]
    fn feature_branch_adds_slug_metadata() {
        // Oracle: #5790 §1 — feature branch adds +<branch-slug>.
        let class = branch_class(Some("ws-h-versioning"), false);
        assert_eq!(
            derive(Some(v(0, 4, 2)), 7, Some("1a2b3c4"), &class),
            "0.4.3-dev.7+g1a2b3c4+ws-h-versioning"
        );
    }

    #[test]
    fn no_tag_falls_back_to_0_1_0_dev_form() {
        // Oracle: #8566 acceptance — with no tag the app reports
        // the 0.1.0-dev fallback form.
        assert_eq!(
            derive(None, 12, Some("abc1234"), &BranchClass::Main),
            "0.1.0-dev.12+gabc1234"
        );
        assert_eq!(derive(None, 0, None, &BranchClass::Main), "0.1.0-dev");
    }

    #[test]
    fn app_tag_parsing_is_scoped_to_the_app() {
        assert_eq!(parse_app_tag(APP, "fleet/v0.1.0"), Some(v(0, 1, 0)));
        // Another app's tag never versions this app.
        assert_eq!(parse_app_tag(APP, "mcpx/v0.1.0"), None);
        assert_eq!(parse_app_tag("mcpx", "fleet/v0.1.0"), None);
        // Malformed tags are rejected, not guessed at.
        assert_eq!(parse_app_tag(APP, "fleet/v0.1"), None);
        assert_eq!(parse_app_tag(APP, "fleet/0.1.0"), None);
        assert_eq!(parse_app_tag(APP, "fleet/v01.2.3"), None);
        assert_eq!(parse_app_tag(APP, "fleet/v0.1.0-rc1"), None);
    }

    #[test]
    fn describe_parsing_handles_long_and_exact_forms() {
        assert_eq!(
            parse_describe(APP, "fleet/v0.4.2-7-g1a2b3c4\n"),
            Some(Describe {
                tag: v(0, 4, 2),
                distance: 7,
                hash: Some("1a2b3c4".to_string()),
            })
        );
        assert_eq!(
            parse_describe(APP, "fleet/v0.4.2"),
            Some(Describe {
                tag: v(0, 4, 2),
                distance: 0,
                hash: None,
            })
        );
        assert_eq!(parse_describe(APP, "mcpx/v0.4.2-7-g1a2b3c4"), None);
        assert_eq!(parse_describe(APP, ""), None);
    }

    #[test]
    fn describe_round_trip_matches_the_policy_example() {
        // The #5790 §1 worked example, end to end.
        let described = parse_describe(APP, "fleet/v0.4.2-7-g1a2b3c4").unwrap();
        let class = branch_class(Some("main"), described.distance == 0);
        assert_eq!(
            derive(
                Some(described.tag),
                described.distance,
                described.hash.as_deref(),
                &class
            ),
            "0.4.3-dev.7+g1a2b3c4"
        );
    }

    #[test]
    fn slugify_sanitizes_branch_names() {
        assert_eq!(
            slugify_branch("queue/14886-8566-derived-versioning"),
            "queue-14886-8566-derived-version"
        );
        assert_eq!(slugify_branch("Feature/Weird_Name!!"), "feature-weird-name");
        assert_eq!(slugify_branch("---"), "feature");
    }

    #[test]
    fn branch_class_defaults_detached_and_main_to_main() {
        assert_eq!(branch_class(None, false), BranchClass::Main);
        assert_eq!(branch_class(Some("main"), false), BranchClass::Main);
        assert_eq!(branch_class(Some("main"), true), BranchClass::Tag);
    }
}
