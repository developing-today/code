//! HTML template rendering.
//!
//! Provides functions for generating HTML responses with proper structure
//! and theme support.

// Allow format string lints - HTML templates need dynamic string building
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::write_with_newline)]

use std::fmt::Write;

use super::content_mode::MediaType;
use super::routes::{FileKind, FileListPage};

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
    html.push_str("\n</head>\n<body class=\"crt-scanlines crt-flicker\">\n");

    // Main content - includes header and footer for SPA navigation
    html.push_str("    <main class=\"min-h-screen\" id=\"main\">\n");
    html.push_str(&render_main_page_wrapper(content));
    html.push_str("    </main>\n");

    html.push_str("</body>\n</html>");

    html
}

/// Render the main page wrapper with header and footer.
/// This is used both for full page renders and partial updates.
pub fn render_main_page_wrapper(content: &str) -> String {
    let mut html = String::with_capacity(2048);

    html.push_str("<div class=\"flex flex-col min-h-screen\">\n");

    // Header - same style as editor inline header
    html.push_str("    <header class=\"inline-header flex items-center justify-between px-3 py-1 bg-base-100 border-b border-base-300 text-xs\" id=\"main-header\">\n");
    html.push_str("        <span class=\"font-bold\"><a href=\"/\" data-nav class=\"crt-bloom\">id</a> <span class=\"text-muted\" id=\"header-subtitle\">// p2p file sharing</span></span>\n");
    html.push_str("        <nav class=\"flex items-center gap-3\">\n");
    html.push_str("            <a href=\"/\" data-nav class=\"crt-glow\">files</a>\n");
    html.push_str("            <a href=\"/peers\" data-nav class=\"crt-glow\">peers</a>\n");
    html.push_str("            <a href=\"/settings\" data-nav class=\"crt-glow\">settings</a>\n");
    html.push_str("            <span class=\"flex items-center gap-1\">\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"sneak\" title=\"Sneak theme\"></button>\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"arch\" title=\"Arch theme\"></button>\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"mech\" title=\"Mech theme\"></button>\n");
    html.push_str("            </span>\n");
    html.push_str("        </nav>\n");
    html.push_str("    </header>\n");

    // Content
    html.push_str("    <div class=\"flex-1\">\n");
    html.push_str("        <div class=\"max-w-4xl mx-auto px-4 py-4\">\n");
    html.push_str(content);
    html.push_str("\n        </div>\n");
    html.push_str("    </div>\n");

    // Footer
    html.push_str("    <footer class=\"inline-footer flex items-center gap-2 px-3 py-1 bg-base-100 border-t border-base-300 text-xs\" id=\"main-footer\">\n");
    html.push_str(
        "        <a href=\"#\" onclick=\"history.back()\" id=\"back-link\" class=\"back-link\">&larr; back</a>",
    );
    html.push_str(" <span class=\"text-base-content/30\">|</span> ");
    html.push_str("<a href=\"/\" class=\"hover:text-primary\" data-nav>id v0.1.0</a>");
    html.push_str(" <span class=\"text-base-content/30\">|</span> ");
    html.push_str("<a href=\"#\" class=\"hover:text-primary\" onclick=\"window.cycleTheme?.(); return false;\"><kbd>Alt+T</kbd> <span>theme</span></a>\n");
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
pub fn render_file_list(page: &FileListPage) -> String {
    let mut html = String::with_capacity(4096);

    // New file form — above the file list, styled like the filter bar
    html.push_str("<div class=\"card bg-base-200 border border-base-300\">");
    html.push_str(
        "<div class=\"px-3 py-2 border-b border-base-300 text-sm font-bold\">New File</div>",
    );
    html.push_str("<div class=\"flex items-center gap-2 px-3 py-2 border-b border-base-300\">");
    html.push_str("<form id=\"new-file-form\" onsubmit=\"window.idApp?.createFile?.(event); return false;\" class=\"contents\">");
    html.push_str("<input type=\"text\" id=\"new-file-name\" name=\"name\" placeholder=\"filename.md\" required class=\"input input-bordered input-xs flex-1 bg-base-100\" />");
    html.push_str(
        "<button type=\"submit\" class=\"btn btn-ghost btn-xs font-mono\">create</button>",
    );
    html.push_str("</form>");
    html.push_str("</div>");
    html.push_str("</div>");

    // File list card
    html.push_str("<div class=\"card bg-base-200 border border-base-300 mt-4\"><div class=\"px-3 py-2 border-b border-base-300 text-sm font-bold\">Files</div>");

    // Search/filter bar — server-side search via JS
    let search_value = page.search.as_deref().map(html_escape).unwrap_or_default();
    let show_deleted_checked = if page.show_deleted { " checked" } else { "" };
    html.push_str("<div class=\"flex items-center gap-2 px-3 py-2 border-b border-base-300\" id=\"file-filter\">");
    let _ = write!(
        html,
        "<input type=\"text\" id=\"file-search\" class=\"input input-bordered input-xs flex-1 bg-base-100\" name=\"search\" \
         placeholder=\"search files & tags...\" autocomplete=\"off\" \
         value=\"{}\" />",
        search_value
    );
    html.push_str("<label class=\"flex items-center gap-1 text-xs whitespace-nowrap cursor-pointer\"><input type=\"checkbox\" id=\"show-auto\" class=\"checkbox checkbox-xs\" /> show auto/archive</label>");
    let _ = write!(
        html,
        "<label class=\"flex items-center gap-1 text-xs whitespace-nowrap cursor-pointer\"><input type=\"checkbox\" id=\"show-deleted\" name=\"show_deleted\" value=\"true\"          class=\"checkbox checkbox-xs\"{} /> show deleted</label>",
        show_deleted_checked
    );
    html.push_str("</div>");

    // Inner content (list + pagination) — replaceable via SPA navigation
    html.push_str("<div id=\"file-list-content\">");
    html.push_str(&render_file_list_content(page));
    html.push_str("</div>");

    html.push_str("</div>"); // card

    html
}

/// Render just the file list items and pagination controls.
///
/// This is the replaceable inner content of the file list card.
/// Used by `/api/files` for search and pagination partial updates.
pub fn render_file_list_content(page: &FileListPage) -> String {
    let files = &page.files;
    let mut html = String::with_capacity(2048);

    if files.is_empty() {
        if page.search.is_some() {
            html.push_str("<p class=\"text-muted p-4\">No files match your search.</p>");
        } else {
            html.push_str("<p class=\"text-muted p-4\">No files stored yet.</p>");
        }
    } else {
        // Bulk action bar (hidden by default, shown when files are selected)
        html.push_str(
            "<div class=\"flex items-center gap-2 px-3 py-2 border-b border-base-300 text-xs\" id=\"bulk-action-bar\" style=\"display:none;\">",
        );
        html.push_str("<span class=\"font-bold\" id=\"bulk-count\">0 selected</span>");
        html.push_str("<input type=\"text\" id=\"bulk-tag-key\" class=\"input input-bordered input-xs w-24 bg-base-100\" placeholder=\"key\" />");
        html.push_str("<input type=\"text\" id=\"bulk-tag-value\" class=\"input input-bordered input-xs w-24 bg-base-100\" placeholder=\"value (optional)\" />");
        html.push_str("<button class=\"btn btn-ghost btn-xs font-mono\" onclick=\"window.idApp?.bulkAddTag?.()\">+ add tag</button>");
        html.push_str("<button class=\"btn btn-ghost btn-xs font-mono text-base-content/50\" onclick=\"window.idApp?.bulkClearSelection?.()\">clear</button>");
        html.push_str("</div>");

        html.push_str("<ul class=\"list-none m-0 p-0\">");
        for file in files {
            let name_escaped = html_escape(&file.name);
            let display_name_escaped = file
                .display_name
                .as_deref()
                .map_or_else(|| name_escaped.clone(), html_escape);
            let hash_escaped = html_escape(&file.hash);
            let short_hash = &file.hash[..12.min(file.hash.len())];

            let kind_attr = match file.kind {
                FileKind::Primary => "primary",
                FileKind::Auto => "auto",
                FileKind::Archive => "archive",
            };

            // Deleted class
            let deleted_class = if file.is_deleted { " file-deleted" } else { "" };

            // Badge text
            let badge = match &file.kind {
                FileKind::Auto => {
                    let parent = file.parent_name.as_deref().unwrap_or("?");
                    format!(
                        "<span class=\"file-badge-auto\">auto: {}</span>",
                        html_escape(parent)
                    )
                }
                FileKind::Archive => {
                    let parent = file.parent_name.as_deref().unwrap_or("?");
                    format!(
                        "<span class=\"file-badge-archive\">archive: {}</span>",
                        html_escape(parent)
                    )
                }
                FileKind::Primary => String::new(),
            };

            // Deleted badge
            let deleted_badge = if file.is_deleted {
                "<span class=\"file-badge-deleted\">deleted</span>".to_owned()
            } else {
                String::new()
            };

            // Tag pills — user-visible tags (values truncated at 256 chars)
            let mut tag_pills = String::new();
            for (key, value) in &file.tags {
                let key_esc = html_escape(key);
                match value {
                    Some(v) => {
                        let truncated = if v.len() > crate::tags::TAG_DISPLAY_MAX_BYTES {
                            format!("{}…", &v[..crate::tags::TAG_DISPLAY_MAX_BYTES])
                        } else {
                            v.clone()
                        };
                        let val_esc = html_escape(&truncated);
                        let _ = write!(
                            tag_pills,
                            "<span class=\"tag-pill\" data-key=\"{}\" data-value=\"{}\">{}: {}</span>",
                            key_esc,
                            html_escape(v),
                            key_esc,
                            val_esc
                        );
                    }
                    None => {
                        let _ = write!(
                            tag_pills,
                            "<span class=\"tag-pill\" data-key=\"{}\">{}</span>",
                            key_esc, key_esc
                        );
                    }
                }
            }
            let tags_html = if tag_pills.is_empty() {
                String::new()
            } else {
                format!("<span class=\"flex gap-1 flex-wrap\">{}</span>", tag_pills)
            };

            // Timestamp display — prefer modified_at, fall back to created_at, then tag-parsed
            let display_ts = file.modified_at.or(file.created_at).or(file.timestamp);
            let date_str = display_ts.map(format_unix_timestamp).unwrap_or_default();
            let date_html = if date_str.is_empty() {
                String::new()
            } else {
                format!(
                    "<span class=\"text-base-content/40 whitespace-nowrap text-xs\">{}</span>",
                    date_str
                )
            };

            // Use /edit/{name} link for primary files, /hash/{hash} for others
            let href = match file.kind {
                FileKind::Primary => format!("/edit/{}", urlencoding::encode(&file.name)),
                _ => format!("/hash/{}", hash_escaped),
            };

            let _ = write!(
                html,
                "<li class=\"file-item flex items-center gap-2 px-3 py-1.5 border-b border-base-300 hover:bg-base-200 text-xs{}\" data-kind=\"{}\" data-name=\"{}\">\
                    <input type=\"checkbox\" class=\"file-select checkbox checkbox-xs\" data-name=\"{}\" />\
                    <span class=\"text-primary/50\">[F]</span>\
                    <a class=\"font-bold hover:text-primary truncate\" href=\"{}\" data-nav>{}</a>\
                    {}{}{}{}\
                    <code class=\"font-mono text-base-content/40\">{}</code>\
                </li>",
                deleted_class,
                kind_attr,
                name_escaped,
                name_escaped,
                href,
                display_name_escaped,
                badge,
                deleted_badge,
                tags_html,
                date_html,
                short_hash,
            );
        }
        html.push_str("</ul>");
    }

    // Pagination controls
    let total_pages = (page.total + page.per_page - 1) / page.per_page.max(1);
    if total_pages > 1 {
        let search_param = page
            .search
            .as_deref()
            .map(|s| format!("&search={}", urlencoding::encode(s)))
            .unwrap_or_default();

        html.push_str("<div class=\"flex items-center justify-between px-3 py-2 border-t border-base-300 text-xs\">");

        // Info text
        let start = (page.page - 1) * page.per_page + 1;
        let end = (start + page.files.len()).saturating_sub(1);
        let _ = write!(
            html,
            "<span class=\"text-base-content/50\">{}-{} of {}</span>",
            start, end, page.total
        );

        html.push_str("<span class=\"flex items-center gap-2\">");

        // Previous button
        if page.page > 1 {
            let _ = write!(
                html,
                "<a class=\"btn btn-ghost btn-xs font-mono\" data-page-nav=\"/api/files?page={}&per_page={}{}\" \
                 data-target=\"#file-list-content\">&laquo; prev</a>",
                page.page - 1,
                page.per_page,
                search_param
            );
        } else {
            html.push_str(
                "<span class=\"btn btn-ghost btn-xs font-mono opacity-30\">&laquo; prev</span>",
            );
        }

        // Page indicator
        let _ = write!(
            html,
            "<span class=\"text-base-content/50\">{} / {}</span>",
            page.page, total_pages
        );

        // Next button
        if page.page < total_pages {
            let _ = write!(
                html,
                "<a class=\"btn btn-ghost btn-xs font-mono\" data-page-nav=\"/api/files?page={}&per_page={}{}\" \
                 data-target=\"#file-list-content\">next &raquo;</a>",
                page.page + 1,
                page.per_page,
                search_param
            );
        } else {
            html.push_str(
                "<span class=\"btn btn-ghost btn-xs font-mono opacity-30\">next &raquo;</span>",
            );
        }

        html.push_str("</span>"); // pagination-btns
        html.push_str("</div>"); // file-pagination
    }

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
pub fn render_editor(doc_id: &str, name: &str, content: &str, hash: &str) -> String {
    let doc_id_escaped = html_escape(doc_id);
    let hash_escaped = html_escape(hash);
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
        "        <span class=\"font-bold truncate\"><a href=\"/\" data-nav>id</a> // <a href=\"{}\" class=\"hover:text-primary\">{}</a></span>\n",
        edit_url, name_escaped
    );
    html.push_str("        <nav class=\"flex items-center gap-3\">\n");
    html.push_str(
        "            <span class=\"editor-status\" id=\"editor-status\">connecting...</span>\n",
    );
    // Save button
    html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"save-btn\" title=\"Save (Ctrl+S)\" onclick=\"window.idApp?.triggerSave?.()\" disabled>save</button>\n");
    // Download dropdown
    html.push_str("            <span class=\"relative\" id=\"download-dropdown\">\n");
    html.push_str("                <button class=\"btn btn-ghost btn-xs font-mono\" id=\"download-btn\" title=\"Download\">dl</button>\n");
    html.push_str("                <span class=\"dropdown-menu\" id=\"download-menu\">\n");
    html.push_str(
        "                    <button class=\"px-3 py-1 text-xs hover:bg-base-300 cursor-pointer text-left whitespace-nowrap\" data-dl-format=\"raw\">raw</button>\n",
    );
    html.push_str("                    <button class=\"px-3 py-1 text-xs hover:bg-base-300 cursor-pointer text-left whitespace-nowrap\" data-dl-format=\"json\">pm json</button>\n");
    let _ = write!(
        html,
        "                    <a class=\"px-3 py-1 text-xs hover:bg-base-300 cursor-pointer text-left whitespace-nowrap block\" href=\"/blob/{}\" download=\"{}\">original</a>\n",
        hash_escaped, name_escaped
    );
    html.push_str("                </span>\n");
    html.push_str("            </span>\n");
    // Rename button
    html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"rename-btn\" title=\"Rename file\" onclick=\"window.idApp?.renameFile?.()\">rename</button>\n");
    // Copy button
    html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"copy-btn\" title=\"Copy file\" onclick=\"window.idApp?.copyFile?.()\">copy</button>\n");
    html.push_str("            <a href=\"/\" data-nav>files</a>\n");
    html.push_str("            <a href=\"/peers\" data-nav>peers</a>\n");
    html.push_str("            <a href=\"/settings\" data-nav>settings</a>\n");
    html.push_str("            <span class=\"flex items-center gap-1\">\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"sneak\" title=\"Sneak theme\"></button>\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"arch\" title=\"Arch theme\"></button>\n");
    html.push_str("                <button class=\"theme-btn\" data-theme=\"mech\" title=\"Mech theme\"></button>\n");
    html.push_str("            </span>\n");
    html.push_str("        </nav>\n");
    html.push_str("    </div>\n");

    // Tag panel — inline tag display and editing for this file
    html.push_str("    <div class=\"flex items-center gap-2 px-3 py-1 bg-base-200 border-b border-base-300 text-xs\" id=\"editor-tag-panel\" data-filename=\"");
    html.push_str(name_urlencoded.as_ref());
    html.push_str("\">\n");
    html.push_str("        <span class=\"font-bold text-base-content/50\">tags:</span>\n");
    html.push_str("        <span class=\"flex gap-1 flex-wrap\" id=\"editor-tag-list\"></span>\n");
    html.push_str("        <span class=\"flex items-center gap-1\" id=\"tag-add-inline\">\n");
    html.push_str("            <input type=\"text\" id=\"tag-add-key\" class=\"input input-bordered input-xs w-20 bg-base-100\" placeholder=\"key\" />\n");
    html.push_str("            <input type=\"text\" id=\"tag-add-value\" class=\"input input-bordered input-xs w-20 bg-base-100\" placeholder=\"value\" />\n");
    html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" onclick=\"window.idApp?.addTagInline?.()\">+</button>\n");
    html.push_str("        </span>\n");
    html.push_str("    </div>\n");

    let _ = write!(
        html,
        "    <div class=\"editor-wrapper\" id=\"editor-container\" data-doc-id=\"{}\" data-filename=\"{}\" data-hash=\"{}\">\n        <div id=\"editor\">{}</div>\n    </div>\n",
        doc_id_escaped, name_urlencoded, hash_escaped, content
    );

    // Inline footer - at end of document
    html.push_str("    <div class=\"editor-inline-footer flex items-center gap-2 px-3 py-1 bg-base-100 border-t border-base-300 text-xs\">\n");
    html.push_str(
        "        <a href=\"#\" id=\"back-link\" class=\"back-link disabled\">&larr; back</a>",
    );
    html.push_str(" <span class=\"text-base-content/30\">|</span> ");
    html.push_str("<a href=\"/\" class=\"hover:text-primary\" data-nav>id v0.1.0</a>");
    html.push_str(" <span class=\"text-base-content/30\">|</span> ");
    html.push_str("<a href=\"#\" class=\"hover:text-primary\" onclick=\"window.cycleTheme?.(); return false;\"><kbd>Alt+T</kbd> <span>theme</span></a>\n");
    html.push_str(" <span class=\"text-base-content/30\">|</span> ");
    html.push_str("<span><kbd>Alt+Z</kbd> <span>wrap</span></span>");
    html.push_str(" <span class=\"text-base-content/30\">|</span> ");
    html.push_str("<span><kbd>Alt+L</kbd> <span>lines</span></span>");
    html.push_str(" <span class=\"text-base-content/30\">|</span> ");
    html.push_str("<span><kbd>Ctrl+F</kbd> <span>find</span></span>");
    html.push_str(" <span class=\"text-base-content/30\">|</span> ");
    html.push_str("<span><kbd>Ctrl+G</kbd> <span>go to line</span></span>\n");
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
pub fn render_editor_page(
    doc_id: &str,
    name: &str,
    content: &str,
    hash: &str,
    assets: &AssetUrls,
) -> String {
    let name_escaped = html_escape(name);
    let editor_content = render_editor(doc_id, name, content, hash);

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
    html.push_str("\n</head>\n<body class=\"crt-scanlines crt-flicker\">\n");
    html.push_str("    <main class=\"min-h-screen editor-main\" id=\"main\">\n");
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
    html.push_str("<div class=\"card bg-base-200 border border-base-300\">\n");
    let _ = write!(
        html,
        "    <div class=\"px-3 py-2 border-b border-base-300 text-sm font-bold\">{}</div>\n",
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
    html.push_str("    <div class=\"flex justify-between mt-4 viewer-actions\" data-filename=\"");
    html.push_str(name_urlencoded.as_ref());
    html.push_str("\">\n");
    html.push_str("        <a href=\"/\" data-nav class=\"text-muted\">&larr; back to files</a>\n");
    html.push_str("        <span class=\"viewer-btns flex items-center gap-2\">\n");
    html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"rename-btn\" title=\"Rename file\" onclick=\"window.idApp?.renameFile?.()\">rename</button>\n");
    html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"copy-btn\" title=\"Copy file\" onclick=\"window.idApp?.copyFile?.()\">copy</button>\n");
    let _ = write!(
        html,
        "            <a href=\"{}\" download=\"{}\" class=\"btn btn-primary btn-xs font-mono\">Download</a>\n",
        blob_url, name_escaped
    );
    html.push_str("        </span>\n");
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
    html.push_str("<div class=\"card bg-base-200 border border-base-300\">\n");
    let _ = write!(
        html,
        "    <div class=\"px-3 py-2 border-b border-base-300 text-sm font-bold\">{}</div>\n",
        name_escaped
    );
    html.push_str("    <div class=\"binary-viewer\">\n");
    html.push_str(
        "        <p class=\"text-muted\">This file cannot be displayed in the browser.</p>\n",
    );
    html.push_str("        <p class=\"text-muted\">Download it to view with an appropriate application.</p>\n");
    html.push_str("    </div>\n");
    html.push_str("    <div class=\"flex justify-between mt-4 viewer-actions\" data-filename=\"");
    html.push_str(name_urlencoded.as_ref());
    html.push_str("\">\n");
    html.push_str("        <a href=\"/\" data-nav class=\"text-muted\">&larr; back to files</a>\n");
    html.push_str("        <span class=\"viewer-btns flex items-center gap-2\">\n");
    html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"rename-btn\" title=\"Rename file\" onclick=\"window.idApp?.renameFile?.()\">rename</button>\n");
    html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"copy-btn\" title=\"Copy file\" onclick=\"window.idApp?.copyFile?.()\">copy</button>\n");
    let _ = write!(
        html,
        "            <a href=\"{}\" download=\"{}\" class=\"btn btn-primary btn-xs font-mono\">Download</a>\n",
        blob_url, name_escaped
    );
    html.push_str("        </span>\n");
    html.push_str("    </div>\n");
    html.push_str("</div>\n");

    html
}

/// Render the settings page.
pub fn render_settings(node_id: &str) -> String {
    let node_id_escaped = html_escape(node_id);

    let mut html = String::with_capacity(2048);
    html.push_str("<div class=\"card bg-base-200 border border-base-300\">\n");
    html.push_str(
        "    <div class=\"px-3 py-2 border-b border-base-300 text-sm font-bold\">Settings</div>\n",
    );
    html.push_str("    <div class=\"p-4\">\n");

    // Display Name section (client identity)
    html.push_str("        <h3>Display Name</h3>\n");
    html.push_str("        <p class=\"text-muted mb-4\">Set a name that appears on your cursor when collaborating.</p>\n");
    html.push_str("        <div class=\"flex gap-2 items-center\">\n");
    html.push_str("            <input type=\"text\" id=\"display-name-input\" class=\"input input-sm input-bordered w-48\" placeholder=\"Anonymous\" />\n");
    html.push_str("            <button id=\"display-name-save\" class=\"btn btn-sm btn-primary\">Save</button>\n");
    html.push_str(
        "            <span id=\"display-name-status\" class=\"text-muted text-sm\"></span>\n",
    );
    html.push_str("        </div>\n");
    html.push_str("        <p id=\"display-name-warning\" class=\"text-warning text-xs mt-1 hidden\">Long names may be truncated.</p>\n");
    html.push_str("        \n");

    html.push_str("        <h3 class=\"mt-8\">Node Identity</h3>\n");
    html.push_str("        <p class=\"text-muted mb-4\">Your node ID is used by peers to connect to you.</p>\n");
    let _ = write!(
        html,
        "        <code class=\"font-mono text-base-content/40 break-all\">{}</code>\n",
        node_id_escaped
    );
    html.push_str("        \n");
    html.push_str("        <h3 class=\"mt-8\">Theme</h3>\n");
    html.push_str("        <p class=\"text-muted mb-4\">Choose your preferred visual theme.</p>\n");
    html.push_str("        <div class=\"flex gap-4\">\n");
    html.push_str("            <button class=\"theme-btn btn btn-sm btn-outline btn-primary\" data-theme=\"sneak\">Sneak</button>\n");
    html.push_str("            <button class=\"theme-btn btn btn-sm btn-outline btn-primary\" data-theme=\"arch\">Arch</button>\n");
    html.push_str("            <button class=\"theme-btn btn btn-sm btn-outline btn-primary\" data-theme=\"mech\">Mech</button>\n");
    html.push_str("        </div>\n");
    html.push_str("        \n");
    html.push_str("        <h3 class=\"mt-8\">Keyboard Shortcuts</h3>\n");
    html.push_str("        <table class=\"table table-xs\">\n");
    html.push_str("            <tr><td><kbd>Alt+T</kbd></td><td>Cycle themes</td></tr>\n");
    html.push_str(
        "            <tr><td><kbd>Ctrl+S</kbd></td><td>Save document (in editor)</td></tr>\n",
    );
    html.push_str("            <tr><td><kbd>Ctrl+Z</kbd></td><td>Undo (in editor)</td></tr>\n");
    html.push_str("            <tr><td><kbd>Ctrl+Y</kbd></td><td>Redo (in editor)</td></tr>\n");
    html.push_str(
        "            <tr><td><kbd>Alt+Z</kbd></td><td>Toggle word wrap (in editor)</td></tr>\n",
    );
    html.push_str(
        "            <tr><td><kbd>Alt+L</kbd></td><td>Toggle line numbers (in editor)</td></tr>\n",
    );
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
    html.push_str("<div id=\"peers-content\" data-auto-refresh=\"10\">\n");
    html.push_str("<div class=\"card bg-base-200 border border-base-300\">\n");
    html.push_str("    <div class=\"px-3 py-2 border-b border-base-300 text-sm font-bold\">Discovered Peers</div>\n");
    html.push_str("    <div class=\"p-4\">\n");
    html.push_str("        <p class=\"text-muted mb-4\">Peers discovered via gossip-based peer discovery.</p>\n");

    if peers.is_empty() {
        html.push_str("        <p class=\"text-muted\">No peers discovered yet.</p>\n");
    } else {
        html.push_str("        <table class=\"table table-xs\">\n");
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
                    <td><code class=\"font-mono text-base-content/40\" title=\"{}\">{}</code></td>\
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

    use crate::web::content_mode::MediaType;
    use crate::web::routes::FileInfo;

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
        let page = FileListPage {
            files: vec![],
            total: 0,
            page: 1,
            per_page: 50,
            search: None,
            show_deleted: false,
        };
        let html = render_file_list(&page);
        assert!(html.contains("No files stored yet"));
    }

    #[test]
    fn test_render_file_list_with_files() {
        let files = vec![FileInfo {
            name: "test.txt".to_owned(),
            display_name: None,
            hash: "abc123def456".to_owned(),
            size: 1024,
            kind: FileKind::Primary,
            parent_name: None,
            timestamp: None,
            created_at: None,
            modified_at: None,
            tags: vec![],
            is_deleted: false,
        }];
        let page = FileListPage {
            total: files.len(),
            files,
            page: 1,
            per_page: 50,
            search: None,
            show_deleted: false,
        };
        let html = render_file_list(&page);
        assert!(html.contains("test.txt"));
        assert!(html.contains("abc123def456"));
    }

    #[test]
    fn test_render_peers_empty() {
        let html = render_peers(&[]);
        assert!(html.contains("No peers discovered yet"));
        assert!(html.contains("Discovered Peers"));
        assert!(html.contains("data-auto-refresh=\"10\""));
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

    // ========================================================================
    // Editor page rename/copy button tests
    // ========================================================================

    #[test]
    fn test_render_editor_has_rename_button() {
        let html = render_editor("abc123", "test.md", "<p>hello</p>", "testhash123");
        assert!(
            html.contains("id=\"rename-btn\""),
            "editor should have rename button"
        );
        assert!(
            html.contains("renameFile"),
            "rename button should call renameFile"
        );
    }

    #[test]
    fn test_render_editor_has_copy_button() {
        let html = render_editor("abc123", "test.md", "<p>hello</p>", "testhash123");
        assert!(
            html.contains("id=\"copy-btn\""),
            "editor should have copy button"
        );
        assert!(
            html.contains("copyFile"),
            "copy button should call copyFile"
        );
    }

    #[test]
    fn test_render_editor_has_data_filename() {
        let html = render_editor("abc123", "test.md", "<p>hello</p>", "testhash123");
        assert!(
            html.contains("data-filename=\"test.md\""),
            "editor should have data-filename attribute"
        );
    }

    #[test]
    fn test_render_editor_escapes_filename() {
        let html = render_editor(
            "abc123",
            "file with spaces.md",
            "<p>content</p>",
            "testhash123",
        );
        // URL-encoded filename in data attribute
        assert!(
            html.contains("data-filename=\"file%20with%20spaces.md\""),
            "editor should URL-encode filename in data attribute"
        );
    }

    // ========================================================================
    // Media viewer rename/copy button tests
    // ========================================================================

    #[test]
    fn test_render_media_viewer_has_rename_button() {
        let html = render_media_viewer("abc123", "photo.jpg", MediaType::Image);
        assert!(
            html.contains("id=\"rename-btn\""),
            "media viewer should have rename button"
        );
        assert!(
            html.contains("renameFile"),
            "rename button should call renameFile"
        );
    }

    #[test]
    fn test_render_media_viewer_has_copy_button() {
        let html = render_media_viewer("abc123", "photo.jpg", MediaType::Image);
        assert!(
            html.contains("id=\"copy-btn\""),
            "media viewer should have copy button"
        );
        assert!(
            html.contains("copyFile"),
            "copy button should call copyFile"
        );
    }

    #[test]
    fn test_render_media_viewer_has_data_filename() {
        let html = render_media_viewer("abc123", "photo.jpg", MediaType::Image);
        assert!(
            html.contains("data-filename=\"photo.jpg\""),
            "media viewer should have data-filename attribute"
        );
    }

    #[test]
    fn test_render_media_viewer_has_viewer_actions_class() {
        let html = render_media_viewer("abc123", "photo.jpg", MediaType::Image);
        assert!(
            html.contains("viewer-actions"),
            "media viewer should have viewer-actions class for JS lookup"
        );
    }

    #[test]
    fn test_render_media_viewer_image() {
        let html = render_media_viewer("abc123", "photo.jpg", MediaType::Image);
        assert!(html.contains("<img"), "image type should render img tag");
        assert!(
            html.contains("media-content"),
            "should have media-content class"
        );
    }

    #[test]
    fn test_render_media_viewer_video() {
        let html = render_media_viewer("abc123", "clip.mp4", MediaType::Video);
        assert!(
            html.contains("<video"),
            "video type should render video tag"
        );
        assert!(html.contains("controls"), "video should have controls");
    }

    #[test]
    fn test_render_media_viewer_audio() {
        let html = render_media_viewer("abc123", "song.mp3", MediaType::Audio);
        assert!(
            html.contains("<audio"),
            "audio type should render audio tag"
        );
        assert!(html.contains("controls"), "audio should have controls");
    }

    #[test]
    fn test_render_media_viewer_pdf() {
        let html = render_media_viewer("abc123", "doc.pdf", MediaType::Pdf);
        assert!(html.contains("<embed"), "pdf type should render embed tag");
        assert!(
            html.contains("application/pdf"),
            "embed should have pdf type"
        );
    }

    #[test]
    fn test_render_media_viewer_download_link() {
        let html = render_media_viewer("abc123", "photo.jpg", MediaType::Image);
        assert!(
            html.contains("Download"),
            "media viewer should have download link"
        );
        assert!(
            html.contains("/blob/abc123"),
            "download should link to blob URL"
        );
    }

    // ========================================================================
    // Binary viewer rename/copy button tests
    // ========================================================================

    #[test]
    fn test_render_binary_viewer_has_rename_button() {
        let html = render_binary_viewer("abc123", "data.bin");
        assert!(
            html.contains("id=\"rename-btn\""),
            "binary viewer should have rename button"
        );
        assert!(
            html.contains("renameFile"),
            "rename button should call renameFile"
        );
    }

    #[test]
    fn test_render_binary_viewer_has_copy_button() {
        let html = render_binary_viewer("abc123", "data.bin");
        assert!(
            html.contains("id=\"copy-btn\""),
            "binary viewer should have copy button"
        );
        assert!(
            html.contains("copyFile"),
            "copy button should call copyFile"
        );
    }

    #[test]
    fn test_render_binary_viewer_has_data_filename() {
        let html = render_binary_viewer("abc123", "data.bin");
        assert!(
            html.contains("data-filename=\"data.bin\""),
            "binary viewer should have data-filename attribute"
        );
    }

    #[test]
    fn test_render_binary_viewer_has_viewer_actions_class() {
        let html = render_binary_viewer("abc123", "data.bin");
        assert!(
            html.contains("viewer-actions"),
            "binary viewer should have viewer-actions class for JS lookup"
        );
    }

    #[test]
    fn test_render_binary_viewer_download_link() {
        let html = render_binary_viewer("abc123", "data.bin");
        assert!(
            html.contains("Download"),
            "binary viewer should have download link"
        );
        assert!(
            html.contains("/blob/abc123"),
            "download should link to blob URL"
        );
    }

    #[test]
    fn test_render_binary_viewer_cannot_display_message() {
        let html = render_binary_viewer("abc123", "data.bin");
        assert!(
            html.contains("cannot be displayed"),
            "binary viewer should show cannot-display message"
        );
    }

    #[test]
    fn test_render_binary_viewer_escapes_filename() {
        let html = render_binary_viewer("abc123", "file<script>.bin");
        // HTML-escaped name in card header
        assert!(
            html.contains("file&lt;script&gt;.bin"),
            "binary viewer should HTML-escape filename in header"
        );
    }
}
