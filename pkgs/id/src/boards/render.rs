//! Portable board theme rendering for web and terminal clients (#6163).
//!
//! This module implements the settled #5858 contract (decision record
//! `doc/2026-11-10_design_id5858.md`) as a pure layer over [`Theme`]
//! and [`BoardView`]:
//!
//! * Decision 2 — the per-token cascade: viewer accessibility >
//!   viewer preference > post > board > platform default, plus the
//!   layout scoping rule (a post theme contributes tokens only; the
//!   board theme's layout governs in-board rendering).
//! * Decision 3, ring 2 — render-time value validation: values for
//!   reserved roles are parsed per role at render; a value that does
//!   not parse drops to the next cascade layer, never reaches an
//!   emitter, and never fails the render.
//! * Decision 3, ring 3 — the accessibility floor: the contrast clamp
//!   (WCAG AA 4.5:1 for text-bearing pairs), the reduced-motion
//!   override, and the unthemeable-chrome rule (a closed
//!   theme-settable role list; everything else is ignored custom data).
//! * Decision 4 — degradation is labelled, never silent: missing
//!   themes, missing or mismatched assets, dropped values, and clamped
//!   pairs each degrade one step and are reported as
//!   [`ThemeAdjustment`]s surfaces can display.
//!
//! The schema splits from #5858 (split 1: `Post::theme_ref`,
//! `Board::foreign_theme_mode`) have not landed: the cascade accepts an
//! optional post-theme input and a local [`ForeignThemeMode`] (default
//! [`ForeignThemeMode::Honor`]) so precedence is implemented and tested
//! before the schema arrives. Suppression of a board theme by the host
//! and personal view layers stays in [`BoardGraph::compose_view`]; this
//! module renders the theme that projection returned.
//!
//! Invariants held (see #5858 §"Invariants implementation must hold"):
//! data, not code (closed value grammars, closed layout set,
//! hash-only assets, no remote fetch); fixed per-token order;
//! non-countermandable accessibility; unthemeable chrome; clamp, never
//! reject; web/terminal parity.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::{ArtifactHash, Theme, ThemeLayout};

/// A theme-settable reserved role and the grammar its values must parse
/// as at render (#5858 Decisions 2–3). Roles outside this list are
/// custom data: renderers ignore them and never error on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReservedRole {
    /// Colour in strict `#rgb` / `#rrggbb` grammar.
    Colour(&'static str),
    /// Length in `px` / `rem` within a bounded range.
    Length(&'static str),
    /// Font family name: letters, digits, spaces, hyphens —
    /// never a path, never a URL.
    Font(&'static str),
    /// Duration in whole milliseconds.
    Duration(&'static str),
}

impl ReservedRole {
    /// Role name as it appears in [`Theme::tokens`].
    const fn name(self) -> &'static str {
        match self {
            Self::Colour(name) | Self::Length(name) | Self::Font(name) | Self::Duration(name) => {
                name
            }
        }
    }
}

/// The closed theme-settable role list. Colour roles follow the #5850
/// tone dimension; the type/motion roles are the cascade's non-colour
/// subset (#5858 Decision 4 `TokensOnly`). Chrome and safety
/// affordances have no entry here by construction.
const RESERVED_ROLES: &[ReservedRole] = &[
    ReservedRole::Colour("surface-base"),
    ReservedRole::Colour("surface-raised"),
    ReservedRole::Colour("surface-sunken"),
    ReservedRole::Colour("content-primary"),
    ReservedRole::Colour("content-secondary"),
    ReservedRole::Colour("content-muted"),
    ReservedRole::Colour("content-link"),
    ReservedRole::Colour("accent"),
    ReservedRole::Colour("accent-content"),
    ReservedRole::Colour("success"),
    ReservedRole::Colour("warning"),
    ReservedRole::Colour("danger"),
    ReservedRole::Colour("info"),
    ReservedRole::Colour("border-subtle"),
    ReservedRole::Colour("border-strong"),
    ReservedRole::Colour("border-focus"),
    ReservedRole::Length("type-body-size"),
    ReservedRole::Font("font-body"),
    ReservedRole::Duration("motion-duration"),
];

/// Look up the reserved role for a token name.
fn reserved_role(name: &str) -> Option<ReservedRole> {
    RESERVED_ROLES
        .iter()
        .find(|role| role.name() == name)
        .copied()
}

/// Whether a reserved role is a colour role (#5858 Decision 4:
/// `TokensOnly` keeps chromatic identity with the board).
const fn is_colour_role(role: ReservedRole) -> bool {
    matches!(role, ReservedRole::Colour(_))
}

/// Text-bearing (foreground, background) pairs the contrast floor
/// checks after the cascade (#5858 Decision 3, ring 3).
const CONTRAST_PAIRS: &[(&str, &str)] = &[
    ("content-primary", "surface-base"),
    ("content-secondary", "surface-base"),
    ("content-muted", "surface-base"),
    ("content-link", "surface-base"),
    ("accent-content", "accent"),
];

/// Minimum contrast ratio for normal text (WCAG AA).
const MIN_CONTRAST: f64 = 4.5;

/// Largest body-text size the length grammar accepts, in pixels.
const MAX_TYPE_PX: f64 = 96.0;
/// Smallest body-text size the length grammar accepts, in pixels.
const MIN_TYPE_PX: f64 = 4.0;
/// Largest motion duration the duration grammar accepts, in milliseconds.
const MAX_MOTION_MS: u64 = 10_000;

/// Parse a strict hex colour (`#rgb` or `#rrggbb`) to sRGB components.
/// Anything else — named colours, `rgb()`, `url(`, markup — fails, so
/// hostile values drop a cascade layer instead of reaching an emitter.
fn parse_colour(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    let bytes = hex.as_bytes();
    let digit = |byte: u8| -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    };
    // One hex digit expanded to a full byte (`f` -> `ff`).
    let expand = |nibble: u8| -> Option<u8> { digit(nibble).map(|d| d * 16 + d) };
    // Two hex digits as one byte.
    let pair = |hi: u8, lo: u8| -> Option<u8> { Some(digit(hi)? * 16 + digit(lo)?) };
    match bytes.len() {
        3 => Some((expand(bytes[0])?, expand(bytes[1])?, expand(bytes[2])?)),
        6 => Some((
            pair(bytes[0], bytes[1])?,
            pair(bytes[2], bytes[3])?,
            pair(bytes[4], bytes[5])?,
        )),
        _ => None,
    }
}

/// Parse a length in `px` or `rem` (1rem = 16px) within the accepted
/// range. Returns pixels.
fn parse_length_px(value: &str) -> Option<f64> {
    let (number, unit) = if let Some(number) = value.strip_suffix("px") {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix("rem") {
        (number, 16.0)
    } else {
        return None;
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let parsed: f64 = number.parse().ok()?;
    if !parsed.is_finite() {
        return None;
    }
    let px = parsed * unit;
    (MIN_TYPE_PX..=MAX_TYPE_PX).contains(&px).then_some(px)
}

/// Parse a font family name: starts with a letter, then letters,
/// digits, spaces, hyphens, underscores. Paths, URLs, quotes, and
/// CSS punctuation fail.
fn parse_font_name(value: &str) -> Option<&str> {
    if value.is_empty() || value.len() > 64 {
        return None;
    }
    let mut chars = value.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_')
    {
        Some(value)
    } else {
        None
    }
}

/// Parse a whole-millisecond duration within the accepted range.
fn parse_duration_ms(value: &str) -> Option<u64> {
    let number = value.strip_suffix("ms")?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parsed: u64 = number.parse().ok()?;
    (parsed <= MAX_MOTION_MS).then_some(parsed)
}

/// Whether a token value parses in its reserved role's grammar.
/// Custom (non-reserved) names are not renderable and return false.
fn value_parses(role: ReservedRole, value: &str) -> bool {
    match role {
        ReservedRole::Colour(_) => parse_colour(value).is_some(),
        ReservedRole::Length(_) => parse_length_px(value).is_some(),
        ReservedRole::Font(_) => parse_font_name(value).is_some(),
        ReservedRole::Duration(_) => parse_duration_ms(value).is_some(),
    }
}

/// WCAG relative luminance of an sRGB colour.
fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// WCAG contrast ratio of two colours.
fn contrast_ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (lighter, darker) = {
        let la = relative_luminance(a);
        let lb = relative_luminance(b);
        if la >= lb { (la, lb) } else { (lb, la) }
    };
    (lighter + 0.05) / (darker + 0.05)
}

/// The renderer's built-in complete theme (#5858 Decision 2, layer 5).
/// Every reserved role has a value, so a conforming render never has
/// an unstyled semantic role; every contrast pair passes the floor by
/// construction (covered by test).
#[must_use]
pub fn platform_default_tokens() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("surface-base", "#1a1d21"),
        ("surface-raised", "#23272e"),
        ("surface-sunken", "#121417"),
        ("content-primary", "#f2f4f7"),
        ("content-secondary", "#c3c9d2"),
        ("content-muted", "#9aa2ad"),
        ("content-link", "#7db4ff"),
        ("accent", "#2f5b9e"),
        ("accent-content", "#ffffff"),
        ("success", "#3d9e5f"),
        ("warning", "#c98a2b"),
        ("danger", "#d0604f"),
        ("info", "#4f9ed0"),
        ("border-subtle", "#3a4048"),
        ("border-strong", "#5a636e"),
        ("border-focus", "#7db4ff"),
        ("type-body-size", "16px"),
        ("font-body", "system-ui"),
        ("motion-duration", "150ms"),
    ])
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect()
}

/// Viewer accessibility overrides: the non-countermandable top cascade
/// layer (#5858 Decision 2, layer 1). No lower layer can countermand,
/// dilute, or rename around these bindings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccessibilityOverrides {
    /// Force a high-contrast palette for all text/surface roles.
    pub high_contrast: bool,
    /// Resolve every motion role to its static equivalent.
    pub reduced_motion: bool,
    /// Floor for the body-text size in pixels.
    pub minimum_type_px: Option<u32>,
}

/// Tokens the accessibility layer binds. Only reserved accessibility
/// roles are set here; board, post, and preference layers cannot
/// supply these bindings through another name.
fn accessibility_tokens(overrides: &AccessibilityOverrides) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    if overrides.high_contrast {
        tokens.insert("surface-base".to_owned(), "#000000".to_owned());
        tokens.insert("surface-raised".to_owned(), "#0a0a0a".to_owned());
        tokens.insert("surface-sunken".to_owned(), "#000000".to_owned());
        tokens.insert("content-primary".to_owned(), "#ffffff".to_owned());
        tokens.insert("content-secondary".to_owned(), "#f5f5f5".to_owned());
        tokens.insert("content-muted".to_owned(), "#e0e0e0".to_owned());
        tokens.insert("content-link".to_owned(), "#ffff00".to_owned());
        tokens.insert("accent".to_owned(), "#ffd700".to_owned());
        tokens.insert("accent-content".to_owned(), "#000000".to_owned());
    }
    if overrides.reduced_motion {
        tokens.insert("motion-duration".to_owned(), "0ms".to_owned());
    }
    tokens
}

/// How a receiving board treats post themes (#5858 Decision 4). Local
/// to this module until the schema split lands `foreign_theme_mode`
/// on the board root; the default is `Honor`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForeignThemeMode {
    /// Post themes enter the cascade at layer 3 in full (tokens;
    /// layout still follows the scoping rule).
    #[default]
    Honor,
    /// Post themes contribute only non-colour roles (type, spacing,
    /// density); colour roles resolve from the board layer down.
    TokensOnly,
    /// Post themes do not enter the cascade; every post renders in
    /// the board theme.
    Flatten,
}

/// Inputs to the per-token cascade for one board render.
#[derive(Clone, Debug, Default)]
pub struct CascadeInputs<'a> {
    /// The board's theme (already suppression-checked by
    /// `compose_view`); `None` degrades one labelled step.
    pub board_theme: Option<&'a Theme>,
    /// A post's theme (its own or its source post's across a repost),
    /// offered to the receiving board's cascade; `None` skips layer 3.
    pub post_theme: Option<&'a Theme>,
    /// The receiving board's foreign-theme mode (default `Honor`).
    pub foreign_mode: ForeignThemeMode,
    /// The viewer's own token overrides (preference layer).
    pub viewer_preferences: BTreeMap<String, String>,
    /// The viewer's accessibility overrides (top layer).
    pub accessibility: AccessibilityOverrides,
}

/// One labelled degradation step (#5858 Decision 4): a missing theme
/// or asset, a dropped value, or a clamped pair degrades exactly one
/// step and never fails the render. Surfaces display [`label`](Self::label).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeAdjustment {
    /// A cascade layer named no theme object; the layer was skipped.
    UnavailableTheme {
        /// Which layer was missing: `"post"` or `"board"`.
        layer: &'static str,
    },
    /// A value failed its role grammar and dropped to the next layer.
    DroppedValue {
        /// Token name.
        role: String,
        /// Cascade layer that supplied the bad value.
        layer: &'static str,
    },
    /// A contrast pair below WCAG AA was clamped to conforming values.
    ClampedContrast {
        /// Foreground role.
        foreground: String,
        /// Background role.
        background: String,
    },
    /// An asset blob was missing, hashed differently, or of an
    /// unsupported kind; the asset was omitted.
    UnavailableAsset {
        /// Content hash of the asset reference.
        hash: String,
    },
}

impl ThemeAdjustment {
    /// Human-readable label surfaces display next to the render.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::UnavailableTheme { .. } => "theme unavailable".to_owned(),
            Self::DroppedValue { .. } => "theme value adjusted".to_owned(),
            Self::ClampedContrast { .. } => "theme adjusted for readability".to_owned(),
            Self::UnavailableAsset { .. } => "asset unavailable".to_owned(),
        }
    }
}

/// A fully resolved theme: effective tokens plus the governing layout
/// and the labelled adjustments taken along the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTheme {
    /// Effective token per reserved role after cascade and floors.
    pub tokens: BTreeMap<String, String>,
    /// Governing layout under the scoping rule.
    pub layout: ThemeLayout,
    /// One entry per degradation step taken.
    pub adjustments: Vec<ThemeAdjustment>,
}

/// Collect one theme layer's contribution: reserved roles whose values
/// parse enter the cascade; unparseable reserved values are reported
/// as dropped (the caller then falls through to the next layer);
/// custom names are ignored silently by design.
fn layer_tokens(
    theme: &Theme,
    layer: &'static str,
    adjustments: &mut Vec<ThemeAdjustment>,
) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    for (name, value) in &theme.tokens {
        let Some(role) = reserved_role(name) else {
            continue;
        };
        if value_parses(role, value) {
            tokens.insert(name.clone(), value.clone());
        } else {
            adjustments.push(ThemeAdjustment::DroppedValue {
                role: name.clone(),
                layer,
            });
        }
    }
    tokens
}

/// Apply the contrast floor to the resolved tokens: every pair below
/// WCAG AA is clamped — first the foreground is replaced from the
/// platform default, then the background too — and recorded. The
/// platform default pairs pass by construction, so clamping always
/// terminates with a passing pair.
fn apply_contrast_floor(
    tokens: &mut BTreeMap<String, String>,
    adjustments: &mut Vec<ThemeAdjustment>,
) {
    let defaults = platform_default_tokens();
    for (foreground, background) in CONTRAST_PAIRS {
        let (Some(fg), Some(bg)) = (tokens.get(*foreground), tokens.get(*background)) else {
            continue;
        };
        let (Some(fg_colour), Some(bg_colour)) = (parse_colour(fg), parse_colour(bg)) else {
            continue;
        };
        if contrast_ratio(fg_colour, bg_colour) >= MIN_CONTRAST {
            continue;
        }
        let default_fg = &defaults[*foreground];
        let default_bg = &defaults[*background];
        let Some(default_fg_colour) = parse_colour(default_fg) else {
            continue;
        };
        if contrast_ratio(default_fg_colour, bg_colour) >= MIN_CONTRAST {
            tokens.insert((*foreground).to_owned(), default_fg.clone());
        } else {
            tokens.insert((*foreground).to_owned(), default_fg.clone());
            tokens.insert((*background).to_owned(), default_bg.clone());
        }
        adjustments.push(ThemeAdjustment::ClampedContrast {
            foreground: (*foreground).to_owned(),
            background: (*background).to_owned(),
        });
    }
}

/// Enforce the minimum type scale over the resolved tokens.
fn apply_minimum_type(tokens: &mut BTreeMap<String, String>, minimum_type_px: Option<u32>) {
    let Some(minimum) = minimum_type_px else {
        return;
    };
    let Some(current) = tokens
        .get("type-body-size")
        .and_then(|value| parse_length_px(value))
    else {
        return;
    };
    if current < f64::from(minimum.clamp(1, 96)) {
        tokens.insert(
            "type-body-size".to_owned(),
            format!("{}px", minimum.clamp(1, 96)),
        );
    }
}

/// Resolve the per-token cascade for a board render (#5858 Decision 2):
/// viewer accessibility > viewer preference > post > board > platform
/// default. Layout follows the scoping rule: the board theme's layout
/// governs; a post theme contributes tokens only.
#[must_use]
pub fn resolve_board_theme(inputs: &CascadeInputs<'_>) -> ResolvedTheme {
    let mut adjustments = Vec::new();
    let mut tokens = platform_default_tokens();

    if let Some(board) = inputs.board_theme {
        for (name, value) in layer_tokens(board, "board", &mut adjustments) {
            tokens.insert(name, value);
        }
    } else {
        adjustments.push(ThemeAdjustment::UnavailableTheme { layer: "board" });
    }

    match (inputs.post_theme, inputs.foreign_mode) {
        (Some(post), ForeignThemeMode::Honor) => {
            for (name, value) in layer_tokens(post, "post", &mut adjustments) {
                tokens.insert(name, value);
            }
        }
        (Some(post), ForeignThemeMode::TokensOnly) => {
            for (name, value) in layer_tokens(post, "post", &mut adjustments) {
                if reserved_role(&name).is_some_and(|role| !is_colour_role(role)) {
                    tokens.insert(name, value);
                }
            }
        }
        (Some(_), ForeignThemeMode::Flatten) => {}
        (None, _) => {
            adjustments.push(ThemeAdjustment::UnavailableTheme { layer: "post" });
        }
    }

    for (name, value) in &inputs.viewer_preferences {
        if let Some(role) = reserved_role(name) {
            if value_parses(role, value) {
                tokens.insert(name.clone(), value.clone());
            } else {
                adjustments.push(ThemeAdjustment::DroppedValue {
                    role: name.clone(),
                    layer: "viewer",
                });
            }
        }
    }

    for (name, value) in accessibility_tokens(&inputs.accessibility) {
        tokens.insert(name, value);
    }
    apply_minimum_type(&mut tokens, inputs.accessibility.minimum_type_px);
    apply_contrast_floor(&mut tokens, &mut adjustments);

    ResolvedTheme {
        tokens,
        layout: inputs
            .board_theme
            .map_or(ThemeLayout::List, |theme| theme.layout),
        adjustments,
    }
}

/// Resolve a standalone post render: outside any board, the post
/// theme's own layout governs, falling back to the platform default
/// layout (#5858 Decision 2 scoping rule).
#[must_use]
pub fn resolve_standalone_post_theme(
    post_theme: Option<&Theme>,
    viewer_preferences: &BTreeMap<String, String>,
    accessibility: &AccessibilityOverrides,
) -> ResolvedTheme {
    let inputs = CascadeInputs {
        board_theme: None,
        post_theme,
        foreign_mode: ForeignThemeMode::Honor,
        viewer_preferences: viewer_preferences.clone(),
        accessibility: accessibility.clone(),
    };
    let mut resolved = resolve_board_theme(&inputs);
    resolved.layout = post_theme.map_or(ThemeLayout::List, |theme| theme.layout);
    resolved
}

/// Verify asset bytes against their content address: the SHA-256 of
/// the bytes must equal the referenced hash (#5858 invariant 6: no
/// remote fetch; assets resolve from the content-addressed store only).
#[must_use]
pub fn verify_asset_bytes(expected: &ArtifactHash, bytes: &[u8]) -> bool {
    sha256_hex(bytes) == expected.as_str()
}

/// Lower-case hex SHA-256 of `bytes` (same canonical form as the
/// package-hash helper elsewhere in this module).
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Asset kinds the renderers can safely present. Anything else —
/// executables, documents, archives — is unsupported and omitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    /// Raster image (PNG, JPEG, GIF, WebP).
    Image,
    /// Font (WOFF, WOFF2, TrueType, OpenType).
    Font,
}

/// Sniff an asset kind from magic bytes. Returns `None` for
/// unsupported or unrecognised kinds.
#[must_use]
pub fn sniff_asset_kind(bytes: &[u8]) -> Option<AssetKind> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()))
    {
        Some(AssetKind::Image)
    } else if bytes.starts_with(b"wOFF")
        || bytes.starts_with(b"wOF2")
        || bytes.starts_with(b"\x00\x01\x00\x00")
        || bytes.starts_with(b"OTTO")
    {
        Some(AssetKind::Font)
    } else {
        None
    }
}

/// A verified theme asset ready for the emitters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedAsset {
    /// Content hash of the asset reference.
    pub hash: ArtifactHash,
    /// Sniffed kind.
    pub kind: AssetKind,
}

/// Resolve a theme's asset references through a caller-supplied
/// content store: each reference must be present, hash-verified, and
/// of a supported kind. Anything else degrades one labelled step and
/// the asset is omitted — the render never fails for an asset.
pub fn resolve_assets(
    assets: &[ArtifactHash],
    fetch: &dyn Fn(&ArtifactHash) -> Option<Vec<u8>>,
) -> (Vec<VerifiedAsset>, Vec<ThemeAdjustment>) {
    let mut verified = Vec::new();
    let mut adjustments = Vec::new();
    for hash in assets {
        let usable = fetch(hash)
            .filter(|bytes| verify_asset_bytes(hash, bytes))
            .and_then(|bytes| sniff_asset_kind(&bytes).map(|kind| (kind, hash)));
        match usable {
            Some((kind, hash)) => verified.push(VerifiedAsset {
                hash: hash.clone(),
                kind,
            }),
            None => adjustments.push(ThemeAdjustment::UnavailableAsset {
                hash: hash.as_str().to_owned(),
            }),
        }
    }
    (verified, adjustments)
}

/// Escape text for HTML embedding: theme and board strings are data
/// and must never parse as markup, script, or style.
#[must_use]
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Strip every control character (including ESC) from untrusted text
/// for terminal embedding: theme strings must never emit terminal
/// escapes. Newlines are removed too — fields are single-line.
#[must_use]
pub fn sanitize_terminal_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Map a validated hex colour to the nearest ANSI 256-colour palette
/// entry through the closed 6×6×6 cube plus grayscale ramp. The input
/// must have passed [`parse_colour`]; anything else maps to white.
#[must_use]
pub fn hex_to_ansi256(hex: &str) -> u8 {
    let Some((r, g, b)) = parse_colour(hex) else {
        return 15;
    };
    let level = |channel: u8| {
        let scaled = u16::from(channel) * 5 / 255;
        u8::try_from(scaled.min(5)).unwrap_or(5)
    };
    let (r6, g6, b6) = (level(r), level(g), level(b));
    let cube_distance = |entry: u8| {
        let e = u32::from(entry);
        let (er, eg, eb) = (e / 36, (e % 36) / 6, e % 6);
        let delta = |a: u32, p: u8| {
            let pa = f64::from(p) * 255.0 / 5.0;
            (f64::from(a) - pa).powi(2)
        };
        delta(u32::from(r), u8::try_from(er).unwrap_or(0))
            + delta(u32::from(g), u8::try_from(eg).unwrap_or(0))
            + delta(u32::from(b), u8::try_from(eb).unwrap_or(0))
    };
    let cube_entry = 16 + r6 * 36 + g6 * 6 + b6;
    let gray_value = (f64::from(r) * 0.299 + f64::from(g) * 0.587 + f64::from(b) * 0.114).round();
    let gray_index = ((gray_value - 8.0) / 10.0).round().clamp(0.0, 23.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let gray_entry = 232 + gray_index as u8;
    let gray_level = 8.0 + 10.0 * f64::from(gray_entry - 232);
    let gray_distance = (f64::from(r) - gray_level).powi(2)
        + (f64::from(g) - gray_level).powi(2)
        + (f64::from(b) - gray_level).powi(2);
    if gray_distance < cube_distance(cube_entry) {
        gray_entry
    } else {
        cube_entry
    }
}

/// Board content the emitters frame with the resolved theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardCard<'a> {
    /// Board display name (untrusted text: escaped/stripped at emit).
    pub board_name: &'a str,
    /// Short description (untrusted text).
    pub blurb: Option<&'a str>,
    /// Items to arrange under the governing layout (untrusted titles).
    pub items: &'a [BoardCardItem<'a>],
}

/// One item in a board card (for example a post title).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardCardItem<'a> {
    /// Item title (untrusted text).
    pub title: &'a str,
    /// Item metadata line (untrusted text).
    pub meta: Option<&'a str>,
}

/// Closed layout-to-class mapping for the web emitter: the layout enum
/// selects a class, never markup or styles from theme data.
const fn layout_class(layout: ThemeLayout) -> &'static str {
    match layout {
        ThemeLayout::List => "board-theme-list",
        ThemeLayout::Grid => "board-theme-grid",
        ThemeLayout::Compact => "board-theme-compact",
    }
}

/// Render a board card as safe HTML: reserved tokens become CSS custom
/// properties (`--theme-<role>`) only — names from the closed list,
/// values grammar-validated, so theme data cannot inject CSS — and all
/// text is HTML-escaped, so theme data cannot inject markup or script.
#[must_use]
pub fn render_web(resolved: &ResolvedTheme, card: &BoardCard<'_>) -> String {
    let mut out = String::new();
    out.push_str("<section class=\"board-theme ");
    out.push_str(layout_class(resolved.layout));
    out.push_str("\" style=\"");
    for role in RESERVED_ROLES {
        if let Some(value) = resolved.tokens.get(role.name()) {
            let _ = write!(out, "--theme-{}:{};", role.name(), value);
        }
    }
    out.push_str("\">\n<h2>");
    out.push_str(&escape_html(card.board_name));
    out.push_str("</h2>\n");
    if let Some(blurb) = card.blurb {
        out.push_str("<p>");
        out.push_str(&escape_html(blurb));
        out.push_str("</p>\n");
    }
    out.push_str("<ul>\n");
    for item in card.items {
        out.push_str("<li><span>");
        out.push_str(&escape_html(item.title));
        out.push_str("</span>");
        if let Some(meta) = item.meta {
            out.push_str(" <small>");
            out.push_str(&escape_html(meta));
            out.push_str("</small>");
        }
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n</section>");
    out
}

/// Render a board card as terminal text: colours map through the
/// closed hex→ANSI-256 function and every untrusted string is stripped
/// of control characters, so theme data cannot emit terminal escapes.
/// Layout governs the item arrangement: list (one per line), compact
/// (dense, truncated), grid (two columns).
#[must_use]
pub fn render_terminal(resolved: &ResolvedTheme, card: &BoardCard<'_>) -> String {
    let colour = |role: &str| {
        resolved
            .tokens
            .get(role)
            .map_or(15, |hex| hex_to_ansi256(hex))
    };
    let (accent, primary, muted) = (
        colour("accent"),
        colour("content-primary"),
        colour("content-muted"),
    );
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\x1b[38;5;{accent}m{board}\x1b[0m",
        board = sanitize_terminal_text(card.board_name),
    );
    if let Some(blurb) = card.blurb {
        let _ = writeln!(
            out,
            "\x1b[38;5;{muted}m{}\x1b[0m",
            sanitize_terminal_text(blurb)
        );
    }
    match resolved.layout {
        ThemeLayout::List => {
            for item in card.items {
                let _ = writeln!(
                    out,
                    "\x1b[38;5;{primary}m- {}\x1b[0m",
                    sanitize_terminal_text(item.title),
                );
                if let Some(meta) = item.meta {
                    let _ = writeln!(
                        out,
                        "  \x1b[38;5;{muted}m{}\x1b[0m",
                        sanitize_terminal_text(meta)
                    );
                }
            }
        }
        ThemeLayout::Compact => {
            for item in card.items {
                let mut title = sanitize_terminal_text(item.title);
                if title.chars().count() > 48 {
                    title = title.chars().take(47).collect::<String>() + "…";
                }
                let _ = writeln!(out, "\x1b[38;5;{primary}m{title}\x1b[0m");
            }
        }
        ThemeLayout::Grid => {
            let titles: Vec<String> = card
                .items
                .iter()
                .map(|item| sanitize_terminal_text(item.title))
                .collect();
            let width = titles
                .iter()
                .map(|t| t.chars().count())
                .max()
                .unwrap_or(0)
                .min(38)
                + 2;
            for pair in titles.chunks(2) {
                let _ = write!(
                    out,
                    "\x1b[38;5;{primary}m{:<width$}\x1b[0m",
                    pair[0],
                    width = width
                );
                if let Some(second) = pair.get(1) {
                    let _ = write!(out, "\x1b[38;5;{primary}m{second}\x1b[0m");
                }
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod render_tests {
    use proptest::prelude::*;

    use super::*;

    fn theme(tokens: &[(&str, &str)], layout: ThemeLayout) -> Theme {
        Theme {
            name: "test theme".to_owned(),
            tokens: tokens
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            layout,
            assets: Vec::new(),
            forked_from: None,
        }
    }

    fn inputs<'a>(
        board: Option<&'a Theme>,
        post: Option<&'a Theme>,
        mode: ForeignThemeMode,
    ) -> CascadeInputs<'a> {
        CascadeInputs {
            board_theme: board,
            post_theme: post,
            foreign_mode: mode,
            viewer_preferences: BTreeMap::new(),
            accessibility: AccessibilityOverrides::default(),
        }
    }

    #[test]
    fn platform_default_is_complete_and_passes_the_floor() {
        let tokens = platform_default_tokens();
        for role in RESERVED_ROLES {
            assert!(tokens.contains_key(role.name()), "missing {role:?}");
        }
        for (foreground, background) in CONTRAST_PAIRS {
            let fg = parse_colour(&tokens[*foreground]).unwrap();
            let bg = parse_colour(&tokens[*background]).unwrap();
            assert!(
                contrast_ratio(fg, bg) >= MIN_CONTRAST,
                "{foreground} on {background} fails the floor"
            );
        }
    }

    #[test]
    fn no_theme_renders_platform_default_with_unavailable_labels() {
        let resolved = resolve_board_theme(&inputs(None, None, ForeignThemeMode::Honor));
        assert_eq!(resolved.tokens, platform_default_tokens());
        assert_eq!(resolved.layout, ThemeLayout::List);
        assert!(
            resolved
                .adjustments
                .contains(&ThemeAdjustment::UnavailableTheme { layer: "board" })
        );
        assert!(
            resolved
                .adjustments
                .contains(&ThemeAdjustment::UnavailableTheme { layer: "post" })
        );
    }

    #[test]
    fn cascade_order_is_viewer_over_post_over_board_over_default() {
        let board = theme(&[("accent", "#111111")], ThemeLayout::List);
        let post = theme(&[("accent", "#222222")], ThemeLayout::Grid);
        let mut preferences = BTreeMap::new();
        preferences.insert("accent".to_owned(), "#333333".to_owned());
        let with_pref = CascadeInputs {
            board_theme: Some(&board),
            post_theme: Some(&post),
            foreign_mode: ForeignThemeMode::Honor,
            viewer_preferences: preferences,
            accessibility: AccessibilityOverrides::default(),
        };
        assert_eq!(resolve_board_theme(&with_pref).tokens["accent"], "#333333");
        let no_pref = inputs(Some(&board), Some(&post), ForeignThemeMode::Honor);
        assert_eq!(resolve_board_theme(&no_pref).tokens["accent"], "#222222");
        let board_only = inputs(Some(&board), None, ForeignThemeMode::Honor);
        let resolved = resolve_board_theme(&board_only);
        assert_eq!(resolved.tokens["accent"], "#111111");
        // Board layout governs in-board rendering even with a post theme.
        assert_eq!(resolve_board_theme(&no_pref).layout, ThemeLayout::List);
    }

    #[test]
    fn accessibility_layer_is_non_countermandable() {
        let board = theme(
            &[
                ("content-primary", "#111111"),
                ("motion-duration", "9000ms"),
            ],
            ThemeLayout::List,
        );
        let mut preferences = BTreeMap::new();
        preferences.insert("content-primary".to_owned(), "#222222".to_owned());
        let resolved = resolve_board_theme(&CascadeInputs {
            board_theme: Some(&board),
            post_theme: None,
            foreign_mode: ForeignThemeMode::Honor,
            viewer_preferences: preferences,
            accessibility: AccessibilityOverrides {
                high_contrast: true,
                reduced_motion: true,
                minimum_type_px: Some(20),
            },
        });
        assert_eq!(resolved.tokens["content-primary"], "#ffffff");
        assert_eq!(resolved.tokens["motion-duration"], "0ms");
        let size_px = parse_length_px(&resolved.tokens["type-body-size"]).unwrap();
        assert!(size_px >= 20.0);
    }

    #[test]
    fn tokens_only_mode_keeps_chromatic_identity_with_the_board() {
        let board = theme(
            &[("accent", "#111111"), ("type-body-size", "18px")],
            ThemeLayout::List,
        );
        let post = theme(
            &[("accent", "#222222"), ("type-body-size", "20px")],
            ThemeLayout::Grid,
        );
        let resolved = resolve_board_theme(&inputs(
            Some(&board),
            Some(&post),
            ForeignThemeMode::TokensOnly,
        ));
        assert_eq!(resolved.tokens["accent"], "#111111");
        assert_eq!(resolved.tokens["type-body-size"], "20px");
        assert_eq!(resolved.layout, ThemeLayout::List);
    }

    #[test]
    fn flatten_mode_ignores_the_post_theme() {
        let board = theme(&[("accent", "#111111")], ThemeLayout::List);
        let post = theme(&[("accent", "#222222")], ThemeLayout::Grid);
        let resolved = resolve_board_theme(&inputs(
            Some(&board),
            Some(&post),
            ForeignThemeMode::Flatten,
        ));
        assert_eq!(resolved.tokens["accent"], "#111111");
    }

    #[test]
    fn unparseable_values_drop_one_layer_with_a_label() {
        let board = theme(
            &[
                ("accent", "url(javascript:alert(1))"),
                ("surface-base", "#23272e"),
            ],
            ThemeLayout::List,
        );
        let resolved = resolve_board_theme(&inputs(Some(&board), None, ForeignThemeMode::Honor));
        assert_eq!(
            resolved.tokens["accent"],
            platform_default_tokens()["accent"]
        );
        assert!(
            resolved
                .adjustments
                .contains(&ThemeAdjustment::DroppedValue {
                    role: "accent".to_owned(),
                    layer: "board",
                })
        );
    }

    #[test]
    fn contrast_trap_is_clamped_and_labelled() {
        let board = theme(
            &[("content-primary", "#ffffff"), ("surface-base", "#ffffff")],
            ThemeLayout::List,
        );
        let resolved = resolve_board_theme(&inputs(Some(&board), None, ForeignThemeMode::Honor));
        let fg = parse_colour(&resolved.tokens["content-primary"]).unwrap();
        let bg = parse_colour(&resolved.tokens["surface-base"]).unwrap();
        assert!(contrast_ratio(fg, bg) >= MIN_CONTRAST);
        assert!(
            resolved
                .adjustments
                .iter()
                .any(|adjustment| matches!(adjustment, ThemeAdjustment::ClampedContrast { .. }))
        );
        assert!(
            resolved
                .adjustments
                .iter()
                .any(|adjustment| adjustment.label() == "theme adjusted for readability")
        );
    }

    #[test]
    fn standalone_post_uses_the_post_layout() {
        let post = theme(&[], ThemeLayout::Grid);
        let resolved = resolve_standalone_post_theme(
            Some(&post),
            &BTreeMap::new(),
            &AccessibilityOverrides::default(),
        );
        assert_eq!(resolved.layout, ThemeLayout::Grid);
        let fallback = resolve_standalone_post_theme(
            None,
            &BTreeMap::new(),
            &AccessibilityOverrides::default(),
        );
        assert_eq!(fallback.layout, ThemeLayout::List);
    }

    #[test]
    fn web_output_keeps_hostile_theme_content_inert() {
        let hostile = theme(
            &[
                ("accent", "#c00000"),
                ("surface-base", "\"><script>alert(1)</script>"),
                ("content-primary", "red; background: url(evil)"),
            ],
            ThemeLayout::Grid,
        );
        let resolved = resolve_board_theme(&inputs(Some(&hostile), None, ForeignThemeMode::Honor));
        // Hostile values fail the grammars and never reach the emitter.
        assert_eq!(
            resolved.tokens["surface-base"],
            platform_default_tokens()["surface-base"]
        );
        let card = BoardCard {
            board_name: "Board <img src=x onerror=alert(1)>",
            blurb: Some("A \"quoted\" blurb & more"),
            items: &[BoardCardItem {
                title: "Post <script>alert(2)</script>",
                meta: Some("meta javascript:evil()"),
            }],
        };
        let html = render_web(&resolved, &card);
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(!html.contains("url("));
        // Structural property: every raw angle bracket in the output
        // belongs to one of the emitter's own tags (section/h2/p/ul/li/
        // span/small open + close = 14). All untrusted text — including
        // "javascript:" in the meta line and the onerror attribute text
        // — survives only as escaped entities, never as markup.
        assert_eq!(html.matches('<').count(), 14);
        assert_eq!(html.matches('>').count(), 14);
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
        assert!(html.contains("meta javascript:evil()"));
        assert!(html.contains("&quot;quoted&quot;"));
        assert!(html.contains("board-theme-grid"));
        assert!(html.contains("--theme-accent:#c00000;"));
    }

    #[test]
    fn terminal_output_strips_escapes_and_keeps_layout() {
        let hostile = theme(&[("accent", "#c00000")], ThemeLayout::Compact);
        let resolved = resolve_board_theme(&inputs(Some(&hostile), None, ForeignThemeMode::Honor));
        let card = BoardCard {
            board_name: "Board \x1b[2J\x1b]0;pwned\x07",
            blurb: None,
            items: &[BoardCardItem {
                title: "\x1b[31mRed\x1b[0m post",
                meta: None,
            }],
        };
        let text = render_terminal(&resolved, &card);
        // The emitter's own SGR sequences wrap sanitized fields: every
        // ESC in the output must open one of our sequences, and no
        // control character from untrusted input may survive. Printable
        // remnants such as "pwned" are inert without an introducer.
        assert!(!text.contains('\x07'));
        let esc_count = text.chars().filter(|c| *c == '\x1b').count();
        let sgr_count = text.matches("\x1b[").count();
        assert_eq!(esc_count, sgr_count);
        // Every remaining control character is the emitter's own: ESC
        // only as an SGR introducer (counted above) and `\n` only as a
        // line separator.
        assert!(
            text.chars()
                .all(|c| !c.is_control() || c == '\x1b' || c == '\n')
        );
        assert!(text.contains("[2J]0;pwned"));
        assert!(text.contains("Board "));
        // Inert SGR residue splits the words, but no introducer
        // survives, so nothing here can restyle the terminal.
        assert!(text.contains("Red"));
        assert!(text.contains("post"));
    }

    #[test]
    fn grid_layout_arranges_two_columns() {
        let board = theme(&[], ThemeLayout::Grid);
        let resolved = resolve_board_theme(&inputs(Some(&board), None, ForeignThemeMode::Honor));
        let card = BoardCard {
            board_name: "B",
            blurb: None,
            items: &[
                BoardCardItem {
                    title: "one",
                    meta: None,
                },
                BoardCardItem {
                    title: "two",
                    meta: None,
                },
            ],
        };
        let text = render_terminal(&resolved, &card);
        let row: Vec<&str> = text.lines().filter(|line| line.contains("one")).collect();
        assert_eq!(row.len(), 1);
        assert!(row[0].contains("two"));
    }

    #[test]
    fn assets_verify_by_content_hash_and_kind() {
        let png = b"\x89PNG\r\n\x1a\nfakepngbytes".to_vec();
        let hash = ArtifactHash::new(sha256_hex(&png)).unwrap();
        let (verified, adjustments) = resolve_assets(&[hash.clone()], &|_| Some(png.clone()));
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].kind, AssetKind::Image);
        assert!(adjustments.is_empty());

        // Wrong bytes for the hash: omitted with a label.
        let (verified, adjustments) = resolve_assets(&[hash.clone()], &|_| Some(b"other".to_vec()));
        assert!(verified.is_empty());
        assert_eq!(adjustments.len(), 1);
        assert_eq!(adjustments[0].label(), "asset unavailable");

        // Missing blob: omitted with a label.
        let (verified, adjustments) = resolve_assets(&[hash], &|_| None);
        assert!(verified.is_empty());
        assert_eq!(adjustments.len(), 1);

        // Right hash, unsupported kind (plain text): omitted.
        let text = b"just text, no magic".to_vec();
        let text_hash = ArtifactHash::new(sha256_hex(&text)).unwrap();
        let (verified, adjustments) = resolve_assets(&[text_hash], &|_| Some(text.clone()));
        assert!(verified.is_empty());
        assert_eq!(adjustments.len(), 1);
    }

    #[test]
    fn font_and_woff_kinds_sniff_correctly() {
        assert_eq!(sniff_asset_kind(b"wOF2rest"), Some(AssetKind::Font));
        assert_eq!(
            sniff_asset_kind(b"\xff\xd8\xffrest"),
            Some(AssetKind::Image)
        );
        assert_eq!(sniff_asset_kind(b"PK\x03\x04"), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// The cascade is per-token with a fixed order: for any subset
        /// of layers supplying a parseable value, the highest layer
        /// wins (#5858 invariant 3).
        #[test]
        fn cascade_order_holds_for_layer_subsets(
            board_on in proptest::bool::ANY,
            post_on in proptest::bool::ANY,
            pref_on in proptest::bool::ANY,
        ) {
            let board = theme(&[("accent", "#111111")], ThemeLayout::List);
            let post = theme(&[("accent", "#222222")], ThemeLayout::List);
            let mut preferences = BTreeMap::new();
            if pref_on {
                preferences.insert("accent".to_owned(), "#333333".to_owned());
            }
            let resolved = resolve_board_theme(&CascadeInputs {
                board_theme: board_on.then_some(&board),
                post_theme: post_on.then_some(&post),
                foreign_mode: ForeignThemeMode::Honor,
                viewer_preferences: preferences,
                accessibility: AccessibilityOverrides::default(),
            });
            let expected = if pref_on {
                "#333333"
            } else if post_on {
                "#222222"
            } else if board_on {
                "#111111"
            } else {
                "#2f5b9e"
            };
            prop_assert_eq!(&resolved.tokens["accent"], expected);
        }

        /// Hostile values never survive to either emitter: any token
        /// value containing markup, CSS-function, or escape characters
        /// either fails its grammar (dropped) or, for text fields, is
        /// escaped/stripped at emit.
        #[test]
        fn hostile_values_never_reach_emitters(
            hostile in "[<>\"'&;()\\\\\x1b]{1,12}",
        ) {
            let evil = format!("{hostile:#?}");
            let theme = theme(
                &[("accent", evil.as_str()), ("font-body", evil.as_str())],
                ThemeLayout::List,
            );
            let resolved = resolve_board_theme(&inputs(Some(&theme), None, ForeignThemeMode::Honor));
            let card = BoardCard {
                board_name: evil.as_str(),
                blurb: Some(evil.as_str()),
                items: &[BoardCardItem { title: evil.as_str(), meta: Some(evil.as_str()) }],
            };
            let html = render_web(&resolved, &card);
            prop_assert!(!html.contains("<script"));
            prop_assert!(!html.contains("javascript:"));
            let text = render_terminal(&resolved, &card);
            // Only the emitter's own SGR introductions may contain ESC.
            let esc_count = text.chars().filter(|c| *c == '\x1b').count();
            let sgr_count = text.matches("\x1b[").count();
            prop_assert_eq!(esc_count, sgr_count);
        }
    }
}
