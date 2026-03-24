//! HTML template rendering.
//!
//! Provides functions for generating HTML responses with proper structure
//! and theme support.

// Allow format string lints - HTML templates need dynamic string building
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::write_with_newline)]

use std::fmt::Write;

use super::content_mode::MediaType;
use super::routes::{FileInfo, FileKind};

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

    // Main content - includes header and footer for HTMX compatibility
    html.push_str("    <main class=\"main\" id=\"main\">\n");
    html.push_str(&render_main_page_wrapper(content));
    html.push_str("    </main>\n");

    html.push_str("</body>\n</html>");

    html
}

/// Render the main page wrapper with header and footer.
/// This is used both for full page renders and HTMX partial updates.
pub fn render_main_page_wrapper(content: &str) -> String {
    let mut html = String::with_capacity(2048);

    html.push_str("<div class=\"main-page\">\n");

    // Header - same style as editor inline header
    html.push_str("    <header class=\"inline-header\" id=\"main-header\">\n");
    html.push_str("        <span class=\"header-title\"><a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\">id</a> <span class=\"text-muted\" id=\"header-subtitle\">// p2p file sharing</span></span>\n");
    html.push_str("        <nav class=\"header-nav\">\n");
    html.push_str("            <a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\">files</a>\n");
    html.push_str("            <a href=\"/peers\" hx-get=\"/peers\" hx-target=\"#main\" hx-push-url=\"true\">peers</a>\n");
    html.push_str("            <a href=\"/settings\" hx-get=\"/settings\" hx-target=\"#main\" hx-push-url=\"true\">settings</a>\n");
    html.push_str("            <span class=\"theme-switcher\">\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"sneak\" title=\"Sneak theme\"></button>\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"arch\" title=\"Arch theme\"></button>\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"mech\" title=\"Mech theme\"></button>\n");
    html.push_str("            </span>\n");
    html.push_str("        </nav>\n");
    html.push_str("    </header>\n");

    // Content
    html.push_str("    <div class=\"main-content\">\n");
    html.push_str("        <div class=\"container\">\n");
    html.push_str(content);
    html.push_str("\n        </div>\n");
    html.push_str("    </div>\n");

    // Footer
    html.push_str("    <footer class=\"inline-footer\" id=\"main-footer\">\n");
    html.push_str(
        "        <a href=\"#\" onclick=\"history.back()\" id=\"back-link\" class=\"back-link\">&larr; back</a>",
    );
    html.push_str(" <span class=\"sep\">|</span> ");
    html.push_str("<a href=\"/\" class=\"footer-link\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\">id v0.1.0</a>");
    html.push_str(" <span class=\"sep\">|</span> ");
    html.push_str("<a href=\"#\" class=\"footer-link\" onclick=\"window.cycleTheme?.(); return false;\"><kbd>Alt+T</kbd> <span>theme</span></a>\n");
    html.push_str("    </footer>\n");

    html.push_str("</div>\n");

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
pub fn render_file_list(files: &[FileInfo]) -> String {
    let mut html = String::with_capacity(4096);

    // New file form — above the file list, styled like the filter bar
    html.push_str("<div class=\"card\">");
    html.push_str("<div class=\"card-header\">New File</div>");
    html.push_str("<div class=\"file-filter\">");
    html.push_str("<form id=\"new-file-form\" onsubmit=\"window.idApp?.createFile?.(event); return false;\" style=\"display: contents;\">");
    html.push_str("<input type=\"text\" id=\"new-file-name\" name=\"name\" placeholder=\"filename.md\" required class=\"file-search\" />");
    html.push_str("<button type=\"submit\" class=\"header-btn\">create</button>");
    html.push_str("</form>");
    html.push_str("</div>");
    html.push_str("</div>");

    // File list card
    html.push_str("<div class=\"card mt-md\"><div class=\"card-header\">Files</div>");

    // Search/filter bar
    html.push_str("<div class=\"file-filter\" id=\"file-filter\">");
    html.push_str("<input type=\"text\" id=\"file-search\" class=\"file-search\" placeholder=\"search files...\" autocomplete=\"off\" />");
    html.push_str("<label class=\"file-toggle\"><input type=\"checkbox\" id=\"show-auto\" /> show auto/archive</label>");
    html.push_str("</div>");

    if files.is_empty() {
        html.push_str("<p class=\"text-muted\" style=\"padding: 1rem;\">No files stored yet.</p>");
    } else {
        html.push_str("<ul class=\"file-list\">");
        for file in files {
            let name_escaped = html_escape(&file.name);
            let hash_escaped = html_escape(&file.hash);
            let short_hash = &file.hash[..12.min(file.hash.len())];

            let kind_attr = match file.kind {
                FileKind::Primary => "primary",
                FileKind::Auto => "auto",
                FileKind::Archive => "archive",
            };

            // Badge text
            let badge = match &file.kind {
                FileKind::Auto => {
                    let parent = file.parent_name.as_deref().unwrap_or("?");
                    format!(
                        "<span class=\"file-badge auto\">auto: {}</span>",
                        html_escape(parent)
                    )
                }
                FileKind::Archive => {
                    let parent = file.parent_name.as_deref().unwrap_or("?");
                    format!(
                        "<span class=\"file-badge archive\">archive: {}</span>",
                        html_escape(parent)
                    )
                }
                FileKind::Primary => String::new(),
            };

            // Timestamp display — prefer modified_at (from MetaDoc), fall back to tag-parsed timestamp
            let display_ts = file.modified_at.or(file.created_at).or(file.timestamp);
            let date_str = display_ts.map(format_unix_timestamp).unwrap_or_default();
            let date_html = if date_str.is_empty() {
                String::new()
            } else {
                format!("<span class=\"file-date\">{}</span>", date_str)
            };

            // Use /file/{name} link for primary files, /edit/{hash} for others
            let href = match file.kind {
                FileKind::Primary => format!("/file/{}", urlencoding::encode(&file.name)),
                _ => format!("/edit/{}", hash_escaped),
            };

            let _ = write!(
                html,
                "<li class=\"file-item\" data-kind=\"{}\" data-name=\"{}\">\
                    <span class=\"file-icon\">[F]</span>\
                    <a class=\"file-name\" href=\"{}\" hx-get=\"{}\" hx-target=\"#main\" hx-push-url=\"true\">{}</a>\
                    {}{}\
                    <code class=\"file-hash\">{}</code>\
                </li>",
                kind_attr, name_escaped, href, href, name_escaped, badge, date_html, short_hash,
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
    let name_escaped = html_escape(name);
    // URL-encode the filename for WebSocket query parameter
    let name_urlencoded = urlencoding::encode(name);
    let edit_url = format!("/edit/{}", doc_id_escaped);

    let mut html = String::with_capacity(2048);
    html.push_str("<div class=\"editor-page\">\n");

    // Inline header - in normal flow at top, floats on scroll
    html.push_str("    <div class=\"editor-inline-header\" id=\"editor-header\">\n");
    let _ = write!(
        html,
        "        <span class=\"editor-inline-title\"><a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\">id</a> // <a href=\"{}\" class=\"editor-file-link\">{}</a></span>\n",
        edit_url, name_escaped
    );
    html.push_str("        <nav class=\"header-nav\">\n");
    html.push_str(
        "            <span class=\"editor-status\" id=\"editor-status\">connecting...</span>\n",
    );
    // Save button
    html.push_str("            <button class=\"header-btn\" id=\"save-btn\" title=\"Save (Ctrl+S)\" onclick=\"window.idApp?.saveFile?.()\" disabled>save</button>\n");
    // Download dropdown
    html.push_str("            <span class=\"dropdown\" id=\"download-dropdown\">\n");
    html.push_str("                <button class=\"header-btn\" id=\"download-btn\" title=\"Download\">dl</button>\n");
    html.push_str("                <span class=\"dropdown-menu\" id=\"download-menu\">\n");
    html.push_str(
        "                    <button class=\"dropdown-item\" data-dl-format=\"raw\">raw</button>\n",
    );
    html.push_str("                    <button class=\"dropdown-item\" data-dl-format=\"json\">pm json</button>\n");
    let _ = write!(
        html,
        "                    <a class=\"dropdown-item\" href=\"/blob/{}\" download=\"{}\">original</a>\n",
        doc_id_escaped, name_escaped
    );
    html.push_str("                </span>\n");
    html.push_str("            </span>\n");
    // Rename button
    html.push_str("            <button class=\"header-btn\" id=\"rename-btn\" title=\"Rename file\" onclick=\"window.idApp?.renameFile?.()\">rename</button>\n");
    html.push_str("            <a href=\"/\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\">files</a>\n");
    html.push_str("            <a href=\"/peers\" hx-get=\"/peers\" hx-target=\"#main\" hx-push-url=\"true\">peers</a>\n");
    html.push_str("            <a href=\"/settings\" hx-get=\"/settings\" hx-target=\"#main\" hx-push-url=\"true\">settings</a>\n");
    html.push_str("            <span class=\"theme-switcher\">\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"sneak\" title=\"Sneak theme\"></button>\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"arch\" title=\"Arch theme\"></button>\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"mech\" title=\"Mech theme\"></button>\n");
    html.push_str("            </span>\n");
    html.push_str("        </nav>\n");
    html.push_str("    </div>\n");

    let _ = write!(
        html,
        "    <div class=\"editor-wrapper\" id=\"editor-container\" data-doc-id=\"{}\" data-filename=\"{}\">\n        <div id=\"editor\">{}</div>\n    </div>\n",
        doc_id_escaped, name_urlencoded, content
    );

    // Inline footer - at end of document
    html.push_str("    <div class=\"editor-inline-footer\">\n");
    html.push_str(
        "        <a href=\"#\" id=\"back-link\" class=\"back-link disabled\">&larr; back</a>",
    );
    html.push_str(" <span class=\"sep\">|</span> ");
    html.push_str("<a href=\"/\" class=\"footer-link\" hx-get=\"/\" hx-target=\"#main\" hx-push-url=\"true\">id v0.1.0</a>");
    html.push_str(" <span class=\"sep\">|</span> ");
    html.push_str("<a href=\"#\" class=\"footer-link\" onclick=\"window.cycleTheme?.(); return false;\"><kbd>Alt+T</kbd> <span>theme</span></a>\n");
    html.push_str("    </div>\n");

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
    let name_escaped = html_escape(name);
    let editor_content = render_editor(doc_id, name, content);

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
    html.push_str("    <main class=\"main editor-main\" id=\"main\">\n");
    html.push_str(&editor_content);
    html.push_str("    </main>\n");
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

/// Render the peers page showing discovered peers.
///
/// # Arguments
///
/// * `peers` - List of (`node_id`, `name`, `blob_count`, `age_secs`) tuples
///
/// # Returns
///
/// HTML fragment for the peers page.
pub fn render_peers(peers: &[(String, String, u64, u64)]) -> String {
    let mut html = String::with_capacity(2048);
    html.push_str("<div id=\"peers-content\" hx-get=\"/peers\" hx-trigger=\"every 10s\" hx-select=\"#peers-content\" hx-target=\"this\" hx-swap=\"outerHTML\">\n");
    html.push_str("<div class=\"card\">\n");
    html.push_str("    <div class=\"card-header\">Discovered Peers</div>\n");
    html.push_str("    <div style=\"padding: 1rem;\">\n");
    html.push_str("        <p class=\"text-muted mb-md\">Peers discovered via gossip-based peer discovery.</p>\n");

    if peers.is_empty() {
        html.push_str("        <p class=\"text-muted\">No peers discovered yet.</p>\n");
    } else {
        html.push_str("        <table>\n");
        html.push_str(
            "            <tr><th>Node ID</th><th>Name</th><th>Blobs</th><th>Last Seen</th></tr>\n",
        );
        for (node_id, name, blob_count, age_secs) in peers {
            let node_id_escaped = html_escape(node_id);
            let name_escaped = html_escape(name);
            let short_id = &node_id_escaped[..12.min(node_id_escaped.len())];
            let age_str = format_age(*age_secs);
            let _ = write!(
                html,
                "            <tr>\
                    <td><code class=\"file-hash\" title=\"{}\">{}</code></td>\
                    <td>{}</td>\
                    <td>{}</td>\
                    <td>{}</td>\
                </tr>\n",
                node_id_escaped, short_id, name_escaped, blob_count, age_str,
            );
        }
        html.push_str("        </table>\n");
    }

    html.push_str("    </div>\n");
    html.push_str("</div>\n");
    html.push_str("</div>");

    html
}

/// Format an age in seconds into a human-readable string.
fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Format a unix timestamp as a short date string (YYYY-MM-DD HH:MM).
#[allow(clippy::cast_possible_truncation)]
fn format_unix_timestamp(ts: u64) -> String {
    // Simple UTC conversion without pulling in chrono
    let secs = ts;
    let mins = secs / 60;
    let hours = mins / 60;
    let days_total = hours / 24;

    let hour = hours % 24;
    let minute = mins % 60;

    // Calculate year/month/day from days since epoch
    let mut y: i64 = 1970;
    #[allow(clippy::cast_possible_wrap)] // days since 1970 won't exceed i64::MAX
    let mut remaining = days_total as i64;

    loop {
        let days_in_year: i64 = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            m = i;
            break;
        }
        remaining -= md;
    }
    let day = remaining + 1;
    let month = m + 1;

    format!("{y:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Format a file size in human-readable form.
#[cfg(test)]
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
        let files = vec![FileInfo {
            name: "test.txt".to_owned(),
            hash: "abc123def456".to_owned(),
            size: 1024,
            kind: FileKind::Primary,
            parent_name: None,
            timestamp: None,
            created_at: None,
            modified_at: None,
        }];
        let html = render_file_list(&files);
        assert!(html.contains("test.txt"));
        assert!(html.contains("abc123def456"));
    }

    #[test]
    fn test_render_peers_empty() {
        let html = render_peers(&[]);
        assert!(html.contains("No peers discovered yet"));
        assert!(html.contains("Discovered Peers"));
        assert!(html.contains("hx-trigger=\"every 10s\""));
    }

    #[test]
    fn test_render_peers_with_peers() {
        let peers = vec![(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
            "test-node".to_owned(),
            42,
            30,
        )];
        let html = render_peers(&peers);
        assert!(html.contains("0123456789ab")); // short ID
        assert!(html.contains("test-node"));
        assert!(html.contains("42"));
        assert!(html.contains("30s ago"));
    }

    #[test]
    fn test_format_age() {
        assert_eq!(format_age(5), "5s ago");
        assert_eq!(format_age(90), "1m ago");
        assert_eq!(format_age(7200), "2h ago");
        assert_eq!(format_age(172_800), "2d ago");
    }
}
