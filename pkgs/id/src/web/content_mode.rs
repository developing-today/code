//! Content mode detection for the web editor.
//!
//! Determines how files should be displayed and edited based on their
//! file extension and content.

use std::path::Path;

/// The mode in which content should be displayed/edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentMode {
    /// Media files displayed natively (images, video, audio, PDF).
    Media(MediaType),
    /// `ProseMirror` JSON files - full rich text editor.
    Rich,
    /// Markdown files - full editor with server-side conversion.
    Markdown,
    /// Plain text files - full editor, lines become paragraphs.
    Plain,
    /// Code/config files - editor with no formatting toolbar/shortcuts.
    Raw,
    /// Binary files that cannot be displayed.
    Binary,
}

impl ContentMode {
    /// Returns the mode name as a string for wire protocol.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Media(_) => "media",
            Self::Rich => "rich",
            Self::Markdown => "markdown",
            Self::Plain => "plain",
            Self::Raw => "raw",
            Self::Binary => "binary",
        }
    }

    /// Returns true if this mode uses the collaborative editor.
    #[must_use]
    pub const fn is_editable(&self) -> bool {
        matches!(self, Self::Rich | Self::Markdown | Self::Plain | Self::Raw)
    }

    /// Returns true if this mode should show the formatting toolbar.
    #[must_use]
    pub const fn has_toolbar(&self) -> bool {
        matches!(self, Self::Rich | Self::Markdown | Self::Plain)
    }
}

/// Type of media for native browser rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Image files (png, jpg, gif, webp, svg).
    Image,
    /// Video files (mp4, webm, ogg).
    Video,
    /// Audio files (mp3, wav, ogg).
    Audio,
    /// PDF documents.
    Pdf,
}

impl MediaType {
    /// Returns the MIME type prefix for this media type.
    #[must_use]
    pub const fn mime_prefix(&self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Pdf => "application/pdf",
        }
    }
}

/// Get the MIME content type for a filename based on extension.
///
/// Returns the appropriate Content-Type header value for serving the file.
#[must_use]
pub fn get_content_type(filename: &str) -> &'static str {
    use std::path::Path;

    let extension = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);

    match extension.as_deref() {
        // Images
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("bmp") => "image/bmp",

        // Video
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("ogv") => "video/ogg",
        Some("mov") => "video/quicktime",
        Some("avi") => "video/x-msvideo",

        // Audio
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("aac") => "audio/aac",
        Some("m4a") => "audio/mp4",

        // PDF
        Some("pdf") => "application/pdf",

        // Text/code types
        Some("txt") => "text/plain; charset=utf-8",
        Some("md" | "markdown") => "text/markdown; charset=utf-8",
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs" | "cjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",

        // Default binary
        _ => "application/octet-stream",
    }
}

/// Detect content mode from filename extension.
///
/// # Arguments
///
/// * `filename` - The filename to analyze (can include path).
///
/// # Returns
///
/// The detected content mode based on extension.
#[must_use]
#[allow(clippy::match_same_arms)] // Explicit list of code extensions is intentional for documentation
pub fn detect_mode(filename: &str) -> ContentMode {
    let path = Path::new(filename);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);

    match extension.as_deref() {
        // ProseMirror JSON - check for .pm.json compound extension
        Some("json") if filename.ends_with(".pm.json") => ContentMode::Rich,

        // Markdown
        Some("md" | "markdown") => ContentMode::Markdown,

        // Plain text
        Some("txt") => ContentMode::Plain,

        // Images
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp") => {
            ContentMode::Media(MediaType::Image)
        }

        // Video
        Some("mp4" | "webm" | "ogv" | "mov" | "avi") => ContentMode::Media(MediaType::Video),

        // Audio
        Some("mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a") => {
            ContentMode::Media(MediaType::Audio)
        }

        // PDF
        Some("pdf") => ContentMode::Media(MediaType::Pdf),

        // Code and config files - raw mode
        Some(
            "js" | "ts" | "jsx" | "tsx" | "mjs" | "cjs" | "rs" | "py" | "rb" | "go" | "java" | "c"
            | "cpp" | "h" | "hpp" | "cs" | "swift" | "kt" | "scala" | "php" | "pl" | "sh" | "bash"
            | "zsh" | "fish" | "ps1" | "bat" | "cmd" | "json" | "toml" | "yaml" | "yml" | "xml"
            | "html" | "htm" | "css" | "scss" | "sass" | "less" | "sql" | "graphql" | "proto"
            | "dockerfile" | "makefile" | "cmake" | "gradle" | "ini" | "cfg" | "conf" | "env"
            | "gitignore" | "dockerignore" | "editorconfig" | "prettierrc" | "eslintrc" | "lock"
            | "sum" | "mod",
        ) => ContentMode::Raw,

        // No extension or unknown - default to raw (editable plain text)
        _ => ContentMode::Raw,
    }
}

/// Check if content is valid UTF-8 text.
///
/// Used to determine if a file with unknown extension should be
/// treated as raw text or binary.
#[must_use]
pub const fn is_valid_utf8(content: &[u8]) -> bool {
    std::str::from_utf8(content).is_ok()
}

/// Detect content mode, with fallback to binary if content is not UTF-8.
///
/// # Arguments
///
/// * `filename` - The filename to analyze.
/// * `content` - The file content bytes.
///
/// # Returns
///
/// The detected content mode, falling back to Binary if not valid UTF-8.
#[must_use]
pub fn detect_mode_with_content(filename: &str, content: &[u8]) -> ContentMode {
    let mode = detect_mode(filename);

    // Media and binary modes don't need UTF-8 check
    if matches!(mode, ContentMode::Media(_) | ContentMode::Binary) {
        return mode;
    }

    // For text-based modes, verify content is valid UTF-8
    if is_valid_utf8(content) {
        mode
    } else {
        ContentMode::Binary
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_markdown() {
        assert_eq!(detect_mode("readme.md"), ContentMode::Markdown);
        assert_eq!(detect_mode("CHANGELOG.markdown"), ContentMode::Markdown);
        assert_eq!(detect_mode("path/to/file.md"), ContentMode::Markdown);
    }

    #[test]
    fn test_detect_prosemirror_json() {
        assert_eq!(detect_mode("document.pm.json"), ContentMode::Rich);
        // Regular JSON should be raw
        assert_eq!(detect_mode("config.json"), ContentMode::Raw);
        assert_eq!(detect_mode("package.json"), ContentMode::Raw);
    }

    #[test]
    fn test_detect_plain_text() {
        assert_eq!(detect_mode("notes.txt"), ContentMode::Plain);
        assert_eq!(detect_mode("README.txt"), ContentMode::Plain);
    }

    #[test]
    fn test_detect_images() {
        assert_eq!(
            detect_mode("photo.png"),
            ContentMode::Media(MediaType::Image)
        );
        assert_eq!(
            detect_mode("photo.jpg"),
            ContentMode::Media(MediaType::Image)
        );
        assert_eq!(
            detect_mode("photo.jpeg"),
            ContentMode::Media(MediaType::Image)
        );
        assert_eq!(
            detect_mode("animation.gif"),
            ContentMode::Media(MediaType::Image)
        );
        assert_eq!(
            detect_mode("image.webp"),
            ContentMode::Media(MediaType::Image)
        );
        assert_eq!(
            detect_mode("icon.svg"),
            ContentMode::Media(MediaType::Image)
        );
    }

    #[test]
    fn test_detect_video() {
        assert_eq!(
            detect_mode("video.mp4"),
            ContentMode::Media(MediaType::Video)
        );
        assert_eq!(
            detect_mode("video.webm"),
            ContentMode::Media(MediaType::Video)
        );
    }

    #[test]
    fn test_detect_audio() {
        assert_eq!(
            detect_mode("song.mp3"),
            ContentMode::Media(MediaType::Audio)
        );
        assert_eq!(
            detect_mode("sound.wav"),
            ContentMode::Media(MediaType::Audio)
        );
    }

    #[test]
    fn test_detect_pdf() {
        assert_eq!(
            detect_mode("document.pdf"),
            ContentMode::Media(MediaType::Pdf)
        );
    }

    #[test]
    fn test_detect_code_files() {
        assert_eq!(detect_mode("main.rs"), ContentMode::Raw);
        assert_eq!(detect_mode("index.js"), ContentMode::Raw);
        assert_eq!(detect_mode("style.css"), ContentMode::Raw);
        assert_eq!(detect_mode("Cargo.toml"), ContentMode::Raw);
        assert_eq!(detect_mode("config.yaml"), ContentMode::Raw);
    }

    #[test]
    fn test_detect_unknown_defaults_to_raw() {
        assert_eq!(detect_mode("file.unknown"), ContentMode::Raw);
        assert_eq!(detect_mode("noextension"), ContentMode::Raw);
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(detect_mode("README.MD"), ContentMode::Markdown);
        assert_eq!(
            detect_mode("image.PNG"),
            ContentMode::Media(MediaType::Image)
        );
        assert_eq!(
            detect_mode("video.MP4"),
            ContentMode::Media(MediaType::Video)
        );
    }

    #[test]
    fn test_detect_with_content_binary() {
        // Valid UTF-8
        assert_eq!(
            detect_mode_with_content("file.txt", b"hello world"),
            ContentMode::Plain
        );

        // Invalid UTF-8 should become Binary
        assert_eq!(
            detect_mode_with_content("file.txt", &[0xFF, 0xFE, 0x00, 0x01]),
            ContentMode::Binary
        );

        // Media files don't check UTF-8
        assert_eq!(
            detect_mode_with_content("image.png", &[0x89, 0x50, 0x4E, 0x47]),
            ContentMode::Media(MediaType::Image)
        );
    }

    #[test]
    fn test_content_mode_as_str() {
        assert_eq!(ContentMode::Rich.as_str(), "rich");
        assert_eq!(ContentMode::Markdown.as_str(), "markdown");
        assert_eq!(ContentMode::Plain.as_str(), "plain");
        assert_eq!(ContentMode::Raw.as_str(), "raw");
        assert_eq!(ContentMode::Binary.as_str(), "binary");
        assert_eq!(ContentMode::Media(MediaType::Image).as_str(), "media");
    }

    #[test]
    fn test_is_editable() {
        assert!(ContentMode::Rich.is_editable());
        assert!(ContentMode::Markdown.is_editable());
        assert!(ContentMode::Plain.is_editable());
        assert!(ContentMode::Raw.is_editable());
        assert!(!ContentMode::Binary.is_editable());
        assert!(!ContentMode::Media(MediaType::Image).is_editable());
    }

    #[test]
    fn test_has_toolbar() {
        assert!(ContentMode::Rich.has_toolbar());
        assert!(ContentMode::Markdown.has_toolbar());
        assert!(ContentMode::Plain.has_toolbar());
        assert!(!ContentMode::Raw.has_toolbar());
        assert!(!ContentMode::Binary.has_toolbar());
        assert!(!ContentMode::Media(MediaType::Image).has_toolbar());
    }

    #[test]
    fn test_get_content_type_images() {
        assert_eq!(get_content_type("photo.png"), "image/png");
        assert_eq!(get_content_type("photo.jpg"), "image/jpeg");
        assert_eq!(get_content_type("photo.jpeg"), "image/jpeg");
        assert_eq!(get_content_type("animation.gif"), "image/gif");
        assert_eq!(get_content_type("image.webp"), "image/webp");
        assert_eq!(get_content_type("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn test_get_content_type_video() {
        assert_eq!(get_content_type("video.mp4"), "video/mp4");
        assert_eq!(get_content_type("video.webm"), "video/webm");
        assert_eq!(get_content_type("video.ogv"), "video/ogg");
    }

    #[test]
    fn test_get_content_type_audio() {
        assert_eq!(get_content_type("song.mp3"), "audio/mpeg");
        assert_eq!(get_content_type("sound.wav"), "audio/wav");
        assert_eq!(get_content_type("audio.ogg"), "audio/ogg");
    }

    #[test]
    fn test_get_content_type_text() {
        assert_eq!(get_content_type("readme.txt"), "text/plain; charset=utf-8");
        assert_eq!(
            get_content_type("README.md"),
            "text/markdown; charset=utf-8"
        );
        assert_eq!(get_content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(get_content_type("style.css"), "text/css; charset=utf-8");
    }

    #[test]
    fn test_get_content_type_application() {
        assert_eq!(get_content_type("doc.pdf"), "application/pdf");
        assert_eq!(
            get_content_type("config.json"),
            "application/json; charset=utf-8"
        );
        assert_eq!(
            get_content_type("main.js"),
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn test_get_content_type_unknown() {
        assert_eq!(get_content_type("file.unknown"), "application/octet-stream");
        assert_eq!(get_content_type("noextension"), "application/octet-stream");
    }
}
