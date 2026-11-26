//! Version strings for the Atlas CLI family (#13279 part 2).
//!
//! Every Atlas binary reports `<package version>+g<source revision>`
//! from `--version`, so a build can be traced to the exact source it
//! was built from, and two different packages no longer collide on a
//! bare `0.1.0` (atlas-search vs fleet search). The revision is
//! baked by atlas-core's build script (`ATLAS_SOURCE_REV`); a build
//! with no git history reports `unknown` rather than inventing one.

/// The source revision this atlas-core build was made from: 12 hex
/// chars of the HEAD commit, or `"unknown"`.
pub const SOURCE_REVISION: &str = env!("ATLAS_SOURCE_REV");

/// The `--version` payload for an Atlas binary: the binary's own
/// package version plus the source revision as semver build
/// metadata, e.g. `0.1.0+g145f1906e1d5`.
pub fn version_string(package_version: &str) -> String {
    format!("{package_version}+g{SOURCE_REVISION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Oracle: the `--version` shape #13279 part 2 requires —
    /// package version, `+g`, then the baked revision verbatim.
    #[test]
    fn version_string_appends_baked_revision_as_build_metadata() {
        assert_eq!(version_string("0.1.0"), format!("0.1.0+g{SOURCE_REVISION}"));
    }

    /// Oracle: the build script always bakes *something* — a
    /// revision or the honest `unknown` — never an empty string.
    #[test]
    fn source_revision_is_never_empty() {
        assert!(!SOURCE_REVISION.is_empty());
    }
}
