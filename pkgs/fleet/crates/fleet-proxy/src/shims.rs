//! The materialized PATH shim dir (§4): regenerated from the
//! negotiated services map at every connect/init, so what is on
//! PATH always matches what the broker actually routes — the
//! anti-drift mechanism behind "repo-queue on PATH the moment
//! you're connected."
//!
//! Contents: one argv0 symlink per advertised *client tool*,
//! pointing at the local fleet binary, plus a stamp file recording
//! the map they were built from. Regeneration is conservative:
//! it removes only symlinks it previously created (named in the
//! old stamp), never foreign files, and a sync whose stamp already
//! matches the map and whose links are intact changes nothing —
//! `fleet init`'s no-op is a real no-op.
//!
//! Which tools a service advertises is deliberately a small,
//! explicit table: the client domains whose backend is that
//! service. The search service is NOT broker-routed (it binds its
//! own port), so atlas-search is not advertised here — shims
//! advertise only what the broker routes. And per §8's
//! scope-gravity warning, advertising is not ownership: these are
//! symlinks to the one local binary, nothing more.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use fleet_proxy_proto::ServiceInfo;

const STAMP_FILE: &str = ".services.json";

/// The client tools a routed service puts on PATH.
fn tools_for_service(service_name: &str) -> &'static [&'static str] {
    match service_name {
        // The laptop-agent daemon's client.
        "laptop-agent" => &["laptop-agent"],
        // The queue broker's client.
        "queue-broker" => &["repo-queue"],
        // The secrets broker has no standalone client tool: fleet
        // resolves secrets in-process (fleet-secrets). Nothing to
        // advertise.
        _ => &[],
    }
}

/// The tool set a services map advertises, sorted and deduplicated.
pub fn tools_for_services(services: &BTreeMap<String, ServiceInfo>) -> Vec<String> {
    let mut tools: Vec<String> = services
        .values()
        .flat_map(|s| tools_for_service(&s.name).iter().map(|t| t.to_string()))
        .collect();
    tools.sort();
    tools.dedup();
    tools
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimSync {
    /// Tool names now linked in the dir.
    pub tools: Vec<String>,
    /// True when the sync changed something on disk (false = the
    /// stamp already matched and every link was intact).
    pub changed: bool,
    /// Non-fatal problems (e.g. a foreign regular file occupying a
    /// tool's name — left alone, named here).
    pub warnings: Vec<String>,
}

fn stamp_json(services: &BTreeMap<String, ServiceInfo>, tools: &[String], exe: &Path) -> String {
    serde_json::to_string(&serde_json::json!({
        "services": services,
        "tools": tools,
        "exe": exe.to_string_lossy(),
    }))
    .unwrap_or_default()
}

fn read_stamp(dir: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(dir.join(STAMP_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

fn stamp_tools(stamp: &serde_json::Value) -> Vec<String> {
    stamp
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `name` is a plain file name: a single normal path component,
/// with no separator, no `.`/`..`, and no root. Every name in the fixed
/// tool table qualifies; anything read back from a stamp that does not
/// is not a name this crate ever wrote.
fn is_plain_tool_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

/// Sync `dir` to the shim set `services` advertises, linking each
/// tool at `exe` (the local fleet binary — canonically resolved,
/// so links don't chain through another symlink).
pub fn sync_shims(
    dir: &Path,
    services: &BTreeMap<String, ServiceInfo>,
    exe: &Path,
) -> std::io::Result<ShimSync> {
    let tools = tools_for_services(services);
    let desired_stamp = stamp_json(services, &tools, exe);
    let old_stamp = read_stamp(dir);
    let mut warnings = Vec::new();

    // Fast path: stamp matches and every link is intact → the
    // no-op `fleet init` promises.
    if let Some(old) = &old_stamp {
        let desired_value: serde_json::Value =
            serde_json::from_str(&desired_stamp).unwrap_or_default();
        if *old == desired_value {
            let intact = tools.iter().all(|t| {
                let link = dir.join(t);
                link.is_symlink()
                    && std::fs::read_link(&link)
                        .map(|target| target == exe)
                        .unwrap_or(false)
            });
            if intact {
                return Ok(ShimSync {
                    tools,
                    changed: false,
                    warnings,
                });
            }
        }
    }

    std::fs::create_dir_all(dir)?;
    // Remove previously-managed links the new map no longer
    // advertises. Only names from the old stamp, only symlinks.
    if let Some(old) = &old_stamp {
        for old_tool in stamp_tools(old) {
            if !tools.contains(&old_tool) {
                // The stamp is data on disk, not proof of ownership: a
                // name fleet wrote is always a plain file name from the
                // fixed table, so anything else (a `../` escape, an
                // absolute path) names a foreign file outside this dir
                // and is never removed (#5778).
                if !is_plain_tool_name(&old_tool) {
                    warnings.push(format!(
                        "stamp names {old_tool:?}, which is not a plain tool name; not removing anything for it"
                    ));
                    continue;
                }
                let link = dir.join(&old_tool);
                if link.is_symlink() {
                    let _ = std::fs::remove_file(&link);
                }
            }
        }
    }
    // (Re)create the advertised links.
    for tool in &tools {
        let link = dir.join(tool);
        if link.is_symlink() {
            if std::fs::read_link(&link).map(|t| t == exe).unwrap_or(false) {
                continue;
            }
            std::fs::remove_file(&link)?;
        } else if link.exists() {
            warnings.push(format!(
                "{} exists and is not a symlink; not replacing it",
                link.display()
            ));
            continue;
        }
        std::os::unix::fs::symlink(exe, &link)?;
    }
    std::fs::write(dir.join(STAMP_FILE), desired_stamp)?;
    Ok(ShimSync {
        tools,
        changed: true,
        warnings,
    })
}

/// The canonical path of the running fleet binary, for shim links.
pub fn current_fleet_exe() -> std::io::Result<PathBuf> {
    std::fs::canonicalize(std::env::current_exe()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_of(entries: &[(&str, &str)]) -> BTreeMap<String, ServiceInfo> {
        entries
            .iter()
            .map(|(port, name)| {
                (
                    port.to_string(),
                    ServiceInfo {
                        name: name.to_string(),
                        version: "0.1.0".to_string(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn tools_follow_the_map() {
        let full = map_of(&[
            ("4799", "laptop-agent"),
            ("4800", "secrets-broker"),
            ("4801", "queue-broker"),
        ]);
        assert_eq!(
            tools_for_services(&full),
            vec!["laptop-agent", "repo-queue"]
        );
        let no_queue = map_of(&[("4799", "laptop-agent")]);
        assert_eq!(tools_for_services(&no_queue), vec!["laptop-agent"]);
        let empty = map_of(&[]);
        assert!(tools_for_services(&empty).is_empty());
    }

    #[test]
    fn sync_adds_then_removes_as_the_map_changes() {
        let dir = tempfile_dir::Dir::new();
        let exe = dir.path().join("fleet");
        std::fs::write(&exe, b"fake").unwrap();
        let full = map_of(&[("4799", "laptop-agent"), ("4801", "queue-broker")]);
        let sync1 = sync_shims(dir.path(), &full, &exe).unwrap();
        assert!(sync1.changed);
        assert!(dir.path().join("repo-queue").is_symlink());
        assert!(dir.path().join("laptop-agent").is_symlink());
        // Unchanged map → no-op.
        let sync2 = sync_shims(dir.path(), &full, &exe).unwrap();
        assert!(!sync2.changed);
        // Shrunk map → the stale shim is removed, the kept one stays.
        let small = map_of(&[("4799", "laptop-agent")]);
        let sync3 = sync_shims(dir.path(), &small, &exe).unwrap();
        assert!(sync3.changed);
        assert!(!dir.path().join("repo-queue").exists());
        assert!(dir.path().join("laptop-agent").is_symlink());
    }

    #[test]
    fn sync_never_replaces_a_foreign_file() {
        let dir = tempfile_dir::Dir::new();
        let exe = dir.path().join("fleet");
        std::fs::write(&exe, b"fake").unwrap();
        std::fs::write(dir.path().join("repo-queue"), b"someone else's tool").unwrap();
        let full = map_of(&[("4801", "queue-broker")]);
        let sync = sync_shims(dir.path(), &full, &exe).unwrap();
        assert_eq!(sync.warnings.len(), 1);
        assert_eq!(
            std::fs::read(dir.path().join("repo-queue")).unwrap(),
            b"someone else's tool"
        );
    }

    #[test]
    fn sync_never_removes_through_a_planted_stamp_name() {
        // #5778: stamp entries that escape the shim dir (a `../` name, an
        // absolute path) must not delete the symlinks they point at.
        let outer = tempfile_dir::Dir::new();
        let dir = outer.path().join("shims");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = outer.path().join("fleet");
        std::fs::write(&exe, b"fake").unwrap();
        let victim = outer.path().join("victim-link");
        std::os::unix::fs::symlink(&exe, &victim).unwrap();
        let stamp = format!(
            "{{\"services\":{{}},\"tools\":[\"../victim-link\",\"{}\"],\"exe\":\"{}\"}}",
            victim.display(),
            exe.display()
        );
        std::fs::write(dir.join(STAMP_FILE), stamp).unwrap();
        let sync = sync_shims(&dir, &map_of(&[]), &exe).unwrap();
        assert!(victim.is_symlink(), "the outside symlink must survive");
        assert_eq!(sync.warnings.len(), 2, "{:?}", sync.warnings);
    }
}

/// Minimal tempdir for tests (avoids a dev-dependency for one helper).
#[cfg(test)]
mod tempfile_dir {
    use std::path::PathBuf;

    pub struct Dir(PathBuf);

    impl Dir {
        pub fn new() -> Dir {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "fleet-proxy-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }

        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
