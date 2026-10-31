//! Build script for `id` to handle frontend asset setup.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_owned());
    let web_dist = Path::new(&manifest_dir).join("web").join("dist");

    if env::var("CARGO_FEATURE_WEB").is_ok() && !web_dist.exists() {
        let _ = fs::create_dir_all(&web_dist);
    }
}
