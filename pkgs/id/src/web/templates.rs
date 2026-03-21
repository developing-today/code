//! HTML template rendering.
//!
//! Provides functions for generating HTML responses with proper structure
//! and theme support.

// Allow format string lints - HTML templates need dynamic string building
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::write_with_newline)]

use std::fmt::Write;

use super::content_mode::MediaType;

/// Asset URLs for templates.
///
/// These are resolved from the manifest at startup to support cache busting
/// via content-hashed filenames.
#[derive(Debug, Clone)]
pub struct AssetUrls {
    /// Path to main JavaScript bundle (e.g., `/assets/main.abc123.js`).
    pub main_js: String,
    /// Path to combined CSS styles (e.g., `/assets/styles.def456.css`).
    pub styles_css: String,
}

impl Default for AssetUrls {
    fn default() -> Self {
        Self {
            main_js: "/assets/main.js".to_owned(),
            styles_css: "/assets/styles.css".to_owned(),
        }
    }
}

/// Render a complete HTML page with the standard layout.
///
/// # Arguments
///
/// * `title` - Page title (shown in browser tab)
/// * `content` - HTML content for the main area
/// * `scripts` - Additional script tags to include
/// * `assets` - Asset URLs (use `AssetUrls::default()` if no manifest)
///
/// # Returns
///
/// A complete HTML document as a string.
pub fn render_page(title: &str, content: &str, scripts: &str, assets: &AssetUrls) -> String {
    let title_escaped = html_escape(title);
    let mut html = String::with_capacity(4096);

    html.push_str("<!DOCTYPE html>\n<html lang=\"en\" data-theme=\"sneak\">\n<head>\n");
    html.push_str("    <meta charset=\"UTF-8\">\n");
    html.push_str(
        "    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n",
    );
    let _ = write!(html, "    <title>{} - id</title>\n", title_escaped);
    let _ = write!(
        html,
        "    <link rel=\"stylesheet\" href=\"{}\">\n",
        assets.styles_css
    );
    let _ = write!(
        html,
        "    <script type=\"module\" src=\"{}\"></script>\n",
        assets.main_js
    );
    html.push_str(scripts);
    html.push_str("\n</head>\n<body>\n");

    // Header
    html.push_str("    <header class=\"header\">\n");
    html.push_str("        <div class=\"container flex items-center justify-between\">\n");
    html.push_str("            <h1 class=\"mb-0\">id <span class=\"text-muted\">// p2p file sharing</span></h1>\n");
    html.push_str("            <nav class=\"flex items-center gap-md\">\n");
    html.push_str("                <a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\">files</a>\n");
    html.push_str("                <a href=\"/settings\" hx-get=\"/settings\" hx-target=\"#main\" hx-push-url=\"true\">settings</a>\n");
    html.push_str("                <div class=\"theme-switcher\">\n");
    html.push_str("                    <button class=\"theme-btn\" data-theme=\"sneak\" title=\"Sneak theme\"></button>\n");
    html.push_str("                    <button class=\"theme-btn\" data-theme=\"arch\" title=\"Arch theme\"></button>\n");
    html.push_str("                    <button class=\"theme-btn\" data-theme=\"mech\" title=\"Mech theme\"></button>\n");
    html.push_str("                </div>\n");
    html.push_str("            </nav>\n");
    html.push_str("        </div>\n");
    html.push_str("    </header>\n");

    // Main content
    html.push_str("    <main class=\"main\" id=\"main\">\n");
    html.push_str("        <div class=\"container\">\n");
    html.push_str(content);
    html.push_str("\n        </div>\n");
    html.push_str("    </main>\n");

    // Footer
    html.push_str("    <footer class=\"footer\">\n");
    html.push_str("        <div class=\"container\">\n");
    html.push_str("            <span class=\"text-muted\">id v0.1.0</span>\n");
    html.push_str("            <span class=\"text-muted\"> | </span>\n");
    html.push_str("            <kbd>Alt+T</kbd> <span class=\"text-muted\">cycle themes</span>\n");
    html.push_str("        </div>\n");
    html.push_str("    </footer>\n");
    html.push_str("</body>\n</html>");

    html
}

/// Render the file list view.
///
/// # Arguments
///
/// * `files` - List of (name, hash, size) tuples
///
/// # Returns
///
/// HTML fragment for the file list.
pub fn render_file_list(files: &[(String, String, u64)]) -> String {
    let mut html = String::from("<div class=\"card\"><div class=\"card-header\">Files</div>");

    if files.is_empty() {
        html.push_str("<p class=\"text-muted\" style=\"padding: 1rem;\">No files stored yet.</p>");
    } else {
        html.push_str("<ul class=\"file-list\">");
        for (name, hash, size) in files {
            let name_escaped = html_escape(name);
            let hash_escaped = html_escape(hash);
            let size_formatted = format_size(*size);
            let short_hash = &hash[..12.min(hash.len())];

            let _ = write!(
                html,
                "<li class=\"file-item\">\
                    <span class=\"file-icon\">[F]</span>\
                    <a class=\"file-name\" href=\"/edit/{}\" hx-get=\"/edit/{}\" hx-target=\"#main\" hx-push-url=\"true\">{}</a>\
                    <span class=\"file-size\">{}</span>\
                    <code class=\"file-hash\">{}</code>\
                </li>",
                hash_escaped, hash_escaped, name_escaped, size_formatted, short_hash,
            );
        }
        html.push_str("</ul>");
    }

    html.push_str("</div>");
    html
}

/// Render the editor view for a document.
///
/// # Arguments
///
/// * `doc_id` - Document identifier (usually the hash)
/// * `name` - Human-readable document name (used for mode detection)
/// * `content` - Initial document content (HTML)
///
/// # Returns
///
/// HTML fragment for the editor.
pub fn render_editor(doc_id: &str, name: &str, content: &str) -> String {
    let doc_id_escaped = html_escape(doc_id);
    // URL-encode the filename for WebSocket query parameter
    let name_urlencoded = urlencoding::encode(name);

    let mut html = String::with_capacity(2048);
    html.push_str("<div class=\"editor-page\">\n");
    let _ = write!(
        html,
        "    <div class=\"editor-wrapper\" id=\"editor-container\" data-doc-id=\"{}\" data-filename=\"{}\">\n        <div id=\"editor\">{}</div>\n    </div>\n",
        doc_id_escaped, name_urlencoded, content
    );
    html.push_str("</div>\n");

    html
}

/// Render a complete editor page with custom header.
///
/// # Arguments
///
/// * `doc_id` - Document identifier (hash)
/// * `name` - Human-readable document name
/// * `content` - Initial document content (HTML)
/// * `assets` - Asset URLs
///
/// # Returns
///
/// A complete HTML document for the editor.
pub fn render_editor_page(doc_id: &str, name: &str, content: &str, assets: &AssetUrls) -> String {
    let doc_id_escaped = html_escape(doc_id);
    let name_escaped = html_escape(name);
    let name_urlencoded = urlencoding::encode(name);
    let edit_url = format!("/edit/{}", doc_id_escaped);

    let mut html = String::with_capacity(4096);

    html.push_str("<!DOCTYPE html>\n<html lang=\"en\" data-theme=\"sneak\">\n<head>\n");
    html.push_str("    <meta charset=\"UTF-8\">\n");
    html.push_str(
        "    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n",
    );
    let _ = write!(html, "    <title>{} - id</title>\n", name_escaped);
    let _ = write!(
        html,
        "    <link rel=\"stylesheet\" href=\"{}\">\n",
        assets.styles_css
    );
    let _ = write!(
        html,
        "    <script type=\"module\" src=\"{}\"></script>\n",
        assets.main_js
    );
    html.push_str("\n</head>\n<body>\n");

    // Editor header - hide on scroll down, show on scroll up
    html.push_str("    <header class=\"header editor-page-header\">\n");
    html.push_str("        <div class=\"container flex items-center justify-between\">\n");
    let _ = write!(
        html,
        "            <h1 class=\"mb-0\"><a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\">id</a> <span class=\"text-muted\">// <a href=\"{}\" class=\"editor-file-link\">{}</a></span></h1>\n",
        edit_url, name_escaped
    );
    html.push_str(
        "            <span class=\"editor-status\" id=\"editor-status\">connecting...</span>\n",
    );
    html.push_str("        </div>\n");
    html.push_str("    </header>\n");

    // Main content - editor fills the space
    html.push_str("    <main class=\"main\" id=\"main\">\n");
    let _ = write!(
        html,
        "        <div class=\"editor-page\">\n            <div class=\"editor-wrapper\" id=\"editor-container\" data-doc-id=\"{}\" data-filename=\"{}\">\n                <div id=\"editor\">{}</div>\n            </div>\n        </div>\n",
        doc_id_escaped, name_urlencoded, content
    );
    html.push_str("    </main>\n");

    // Footer
    html.push_str("    <footer class=\"footer editor-page-footer\">\n");
    html.push_str("        <div class=\"container\">\n");
    html.push_str("            <a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\" class=\"text-muted\">&larr; back</a>\n");
    html.push_str("        </div>\n");
    html.push_str("    </footer>\n");
    html.push_str("</body>\n</html>");

    html
}

/// Render the media viewer for images, video, audio, and PDF.
///
/// # Arguments
///
/// * `doc_id` - Document identifier (hash) for blob URL
/// * `name` - Human-readable file name
/// * `media_type` - Type of media to render
///
/// # Returns
///
/// HTML fragment for the media viewer.
pub fn render_media_viewer(doc_id: &str, name: &str, media_type: MediaType) -> String {
    let doc_id_escaped = html_escape(doc_id);
    let name_escaped = html_escape(name);
    let name_urlencoded = urlencoding::encode(name);
    let blob_url = format!("/blob/{}?filename={}", doc_id_escaped, name_urlencoded);

    let mut html = String::with_capacity(1024);
    html.push_str("<div class=\"card\">\n");
    let _ = write!(
        html,
        "    <div class=\"card-header\">{}</div>\n",
        name_escaped
    );
    html.push_str("    <div class=\"media-viewer\">\n");

    match media_type {
        MediaType::Image => {
            let _ = write!(
                html,
                "        <img src=\"{}\" alt=\"{}\" class=\"media-content\" />\n",
                blob_url, name_escaped
            );
        }
        MediaType::Video => {
            let _ = write!(
                html,
                "        <video src=\"{}\" controls class=\"media-content\">\n            Your browser does not support the video tag.\n        </video>\n",
                blob_url
            );
        }
        MediaType::Audio => {
            let _ = write!(
                html,
                "        <audio src=\"{}\" controls class=\"media-content\">\n            Your browser does not support the audio tag.\n        </audio>\n",
                blob_url
            );
        }
        MediaType::Pdf => {
            let _ = write!(
                html,
                "        <embed src=\"{}\" type=\"application/pdf\" class=\"media-content media-pdf\" />\n",
                blob_url
            );
        }
    }

    html.push_str("    </div>\n");
    html.push_str("    <div class=\"flex justify-between mt-md\">\n");
    html.push_str("        <a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\" class=\"text-muted\">&larr; back to files</a>\n");
    let _ = write!(
        html,
        "        <a href=\"{}\" download=\"{}\" class=\"primary\">Download</a>\n",
        blob_url, name_escaped
    );
    html.push_str("    </div>\n");
    html.push_str("</div>\n");

    html
}

/// Render the binary file viewer with download option.
///
/// Shown for files that cannot be displayed in the browser.
///
/// # Arguments
///
/// * `doc_id` - Document identifier (hash) for blob URL
/// * `name` - Human-readable file name
///
/// # Returns
///
/// HTML fragment for the binary viewer.
pub fn render_binary_viewer(doc_id: &str, name: &str) -> String {
    let doc_id_escaped = html_escape(doc_id);
    let name_escaped = html_escape(name);
    let name_urlencoded = urlencoding::encode(name);
    let blob_url = format!("/blob/{}?filename={}", doc_id_escaped, name_urlencoded);

    let mut html = String::with_capacity(512);
    html.push_str("<div class=\"card\">\n");
    let _ = write!(
        html,
        "    <div class=\"card-header\">{}</div>\n",
        name_escaped
    );
    html.push_str("    <div class=\"binary-viewer\">\n");
    html.push_str(
        "        <p class=\"text-muted\">This file cannot be displayed in the browser.</p>\n",
    );
    html.push_str("        <p class=\"text-muted\">Download it to view with an appropriate application.</p>\n");
    html.push_str("    </div>\n");
    html.push_str("    <div class=\"flex justify-between mt-md\">\n");
    html.push_str("        <a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\" class=\"text-muted\">&larr; back to files</a>\n");
    let _ = write!(
        html,
        "        <a href=\"{}\" download=\"{}\" class=\"primary\">Download</a>\n",
        blob_url, name_escaped
    );
    html.push_str("    </div>\n");
    html.push_str("</div>\n");

    html
}

/// Render the settings page.
pub fn render_settings(node_id: &str) -> String {
    let node_id_escaped = html_escape(node_id);

    let mut html = String::with_capacity(2048);
    html.push_str("<div class=\"card\">\n");
    html.push_str("    <div class=\"card-header\">Settings</div>\n");
    html.push_str("    <div style=\"padding: 1rem;\">\n");
    html.push_str("        <h3>Node Identity</h3>\n");
    html.push_str("        <p class=\"text-muted mb-md\">Your node ID is used by peers to connect to you.</p>\n");
    let _ = write!(
        html,
        "        <code class=\"file-hash\" style=\"word-break: break-all;\">{}</code>\n",
        node_id_escaped
    );
    html.push_str("        \n");
    html.push_str("        <h3 class=\"mt-lg\">Theme</h3>\n");
    html.push_str(
        "        <p class=\"text-muted mb-md\">Choose your preferred visual theme.</p>\n",
    );
    html.push_str("        <div class=\"flex gap-md\">\n");
    html.push_str("            <button class=\"theme-btn\" data-theme=\"sneak\">Sneak</button>\n");
    html.push_str("            <button class=\"theme-btn\" data-theme=\"arch\">Arch</button>\n");
    html.push_str("            <button class=\"theme-btn\" data-theme=\"mech\">Mech</button>\n");
    html.push_str("        </div>\n");
    html.push_str("        \n");
    html.push_str("        <h3 class=\"mt-lg\">Keyboard Shortcuts</h3>\n");
    html.push_str("        <table>\n");
    html.push_str("            <tr><td><kbd>Alt+T</kbd></td><td>Cycle themes</td></tr>\n");
    html.push_str(
        "            <tr><td><kbd>Ctrl+S</kbd></td><td>Save document (in editor)</td></tr>\n",
    );
    html.push_str("            <tr><td><kbd>Ctrl+Z</kbd></td><td>Undo (in editor)</td></tr>\n");
    html.push_str("            <tr><td><kbd>Ctrl+Y</kbd></td><td>Redo (in editor)</td></tr>\n");
    html.push_str("        </table>\n");
    html.push_str("    </div>\n");
    html.push_str("</div>");

    html
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Format a file size in human-readable form.
#[allow(clippy::cast_precision_loss)]
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("say \"hello\""), "say &quot;hello&quot;");
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1_048_576), "1.0 MB");
        assert_eq!(format_size(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn test_render_page_structure() {
        let assets = AssetUrls::default();
        let html = render_page("Test", "<p>Content</p>", "", &assets);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>Test - id</title>"));
        assert!(html.contains("<p>Content</p>"));
        assert!(html.contains("data-theme=\"sneak\""));
        assert!(html.contains("/assets/styles.css"));
        assert!(html.contains("/assets/main.js"));
    }

    #[test]
    fn test_render_page_with_hashed_assets() {
        let assets = AssetUrls {
            main_js: "/assets/main.abc12345.js".to_owned(),
            styles_css: "/assets/styles.def67890.css".to_owned(),
        };
        let html = render_page("Test", "<p>Content</p>", "", &assets);
        assert!(html.contains("/assets/main.abc12345.js"));
        assert!(html.contains("/assets/styles.def67890.css"));
    }

    #[test]
    fn test_render_file_list_empty() {
        let html = render_file_list(&[]);
        assert!(html.contains("No files stored yet"));
    }

    #[test]
    fn test_render_file_list_with_files() {
        let files = vec![("test.txt".to_owned(), "abc123def456".to_owned(), 1024)];
        let html = render_file_list(&files);
        assert!(html.contains("test.txt"));
        assert!(html.contains("abc123def456"));
        assert!(html.contains("1.0 KB"));
    }
}
