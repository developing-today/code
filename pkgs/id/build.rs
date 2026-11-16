//! Build script for `id` to handle frontend asset setup.

use std::env;
use std::fs;
use std::path::Path;

fn dist_has_assets(web_dist: &Path) -> bool {
    match fs::read_dir(web_dist) {
        Ok(entries) => entries
            .flatten()
            .any(|entry| entry.file_name() != ".gitkeep"),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::dist_has_assets;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("id-build-test-{name}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_or_gitkeep_only_is_false() {
        let dir = temp_dir("empty");
        assert!(!dist_has_assets(&dir));

        fs::write(dir.join(".gitkeep"), "").unwrap();
        assert!(!dist_has_assets(&dir));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_asset_is_true() {
        let dir = temp_dir("assets");
        fs::write(dir.join(".gitkeep"), "").unwrap();
        fs::write(dir.join("index.html"), "<html></html>").unwrap();
        assert!(dist_has_assets(&dir));

        let _ = fs::remove_dir_all(&dir);
    }
}

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_owned());
    let web_dist = Path::new(&manifest_dir).join("web").join("dist");

    if env::var("CARGO_FEATURE_WEB").is_err() {
        return;
    }

    // Release builds embed whatever `web/dist` holds. Substituting an
    // empty directory there would ship an executable whose web UI
    // 404s every asset (AGENTS.md: the web variant requires the
    // frontend built first), so a release build with missing or
    // empty assets fails explicitly instead (#7152, Codex P1). Dev
    // builds keep the convenience this script exists for: a dummy
    // dist lets `cargo check` / `cargo test` compile on a clean
    // checkout where the frontend has not been built.
    let profile = env::var("PROFILE").unwrap_or_default();
    if profile == "release" {
        assert!(
            dist_has_assets(&web_dist),
            "release build with the `web` feature requires built frontend assets in {}, \
             but it is missing or empty — build them first: \
             cd web && bun install && bun run build",
            web_dist.display()
        );
        return;
    }

    if !web_dist.exists() {
        let _ = fs::create_dir_all(&web_dist);
    }
}
