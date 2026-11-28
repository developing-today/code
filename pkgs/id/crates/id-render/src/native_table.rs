//! The native mapping tables, as data.
//!
//! Every entry is transcribed from decision record #5852
//! (`doc/2026-11-08_decision_id5852_native_rendering_os_standard_controls.md`):
//! the element-kind → OS-standard-control table of §2 and the
//! assertion mapping of §3, for the record's three platforms
//! (macOS, Windows, GNOME). Per #5855 invariant 6 ("tables are
//! data"), a mapping change is a change to these tables, never a
//! change to the lowering code in [`crate::lower_native`].
//!
//! An assertion role with no native mapping on a platform is an
//! explicit **platform-default** entry (§3.1): the `None` rendering
//! in [`RoleRow`] is that marker, and the lowering records it in
//! the coverage report — a role is either mapped or marked, never
//! absent (#5852 §3, invariant 3). Dimension and token names are
//! the closed v1 set of #5850, shared with the web table
//! ([`crate::web_table`]).

/// The native platforms of #5852 §2, by the target names a
/// composed document carries for them.
pub const PLATFORMS: &[&str] = &["mac", "windows", "gnome"];

/// Whether `target` names one of the native platforms.
#[must_use]
pub fn is_platform(target: &str) -> bool {
    PLATFORMS.contains(&target)
}

/// One element-kind → OS-standard-control row of #5852 §2: the
/// standard control for the kind on each platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlRow {
    /// Element kind, e.g. `button`.
    pub kind: &'static str,
    /// macOS standard control (AppKit/SwiftUI).
    pub mac: &'static str,
    /// Windows standard control (WinUI).
    pub windows: &'static str,
    /// GNOME standard widget (GTK 4 / libadwaita).
    pub gnome: &'static str,
}

impl ControlRow {
    /// The control this row selects on `platform`.
    #[must_use]
    pub fn control_on(&self, platform: &str) -> &'static str {
        match platform {
            "windows" => self.windows,
            "gnome" => self.gnome,
            _ => self.mac,
        }
    }
}

/// #5852 §2 rows. Kinds whose control depends on a variant
/// attribute (`field`) are resolved by [`variant_control`]; kinds
/// with no row at all lower to the platform's standard container
/// and are reported (see [`crate::lower_native`]). Rows the record
/// groups (card / media / page-section / master-detail, §2 last
/// block) are spelled out per kind with the record's composition
/// for that kind.
pub const CONTROL_ROWS: &[ControlRow] = &[
    ControlRow {
        kind: "document",
        mac: "window",
        windows: "window",
        gnome: "window",
    },
    ControlRow {
        kind: "page",
        mac: "grouped box composition",
        windows: "card container composition",
        gnome: "card style class composition",
    },
    ControlRow {
        kind: "section",
        mac: "grouped box composition",
        windows: "card container composition",
        gnome: "card style class composition",
    },
    ControlRow {
        kind: "nav",
        mac: "source-list sidebar",
        windows: "NavigationView",
        gnome: "AdwNavigationSplitView sidebar",
    },
    ControlRow {
        kind: "sidebar",
        mac: "source-list sidebar",
        windows: "NavigationView",
        gnome: "AdwNavigationSplitView sidebar",
    },
    ControlRow {
        kind: "heading",
        mac: "label (Dynamic Type text style)",
        windows: "TextBlock",
        gnome: "label (libadwaita typography style class)",
    },
    ControlRow {
        kind: "text",
        mac: "label (Dynamic Type text style)",
        windows: "TextBlock",
        gnome: "label (libadwaita typography style class)",
    },
    ControlRow {
        kind: "strong",
        mac: "label (Dynamic Type text style)",
        windows: "TextBlock",
        gnome: "label (libadwaita typography style class)",
    },
    ControlRow {
        kind: "emphasis",
        mac: "label (Dynamic Type text style)",
        windows: "TextBlock",
        gnome: "label (libadwaita typography style class)",
    },
    ControlRow {
        kind: "card",
        mac: "grouped box",
        windows: "card container",
        gnome: "card style class",
    },
    ControlRow {
        kind: "list",
        mac: "outline view",
        windows: "ListView",
        gnome: "GtkListView",
    },
    ControlRow {
        kind: "item",
        mac: "outline view row",
        windows: "ListViewItem",
        gnome: "GtkListView row",
    },
    ControlRow {
        kind: "table",
        mac: "table view",
        windows: "data grid control",
        gnome: "GtkColumnView",
    },
    ControlRow {
        kind: "row",
        mac: "table view row",
        windows: "data grid row",
        gnome: "column view row",
    },
    ControlRow {
        kind: "cell",
        mac: "table cell view",
        windows: "data grid cell",
        gnome: "column view cell",
    },
    ControlRow {
        kind: "image",
        mac: "image view",
        windows: "Image",
        gnome: "GtkPicture",
    },
    ControlRow {
        kind: "link",
        mac: "link-styled button",
        windows: "HyperlinkButton",
        gnome: "GtkLinkButton",
    },
    ControlRow {
        kind: "master-detail",
        mac: "split view",
        windows: "split layout",
        gnome: "AdwNavigationSplitView",
    },
    ControlRow {
        kind: "detail",
        mac: "split view detail pane",
        windows: "split layout detail pane",
        gnome: "split view detail pane",
    },
    ControlRow {
        kind: "form",
        mac: "grouped form layout",
        windows: "form layout in window/dialog",
        gnome: "AdwPreferencesGroup",
    },
    ControlRow {
        kind: "alert",
        mac: "alert sheet",
        windows: "InfoBar",
        gnome: "AdwBanner",
    },
    ControlRow {
        kind: "button",
        mac: "NSButton (standard push button)",
        windows: "Button",
        gnome: "GtkButton",
    },
    ControlRow {
        kind: "dialog",
        mac: "alert/sheet",
        windows: "ContentDialog",
        gnome: "GtkAlertDialog",
    },
    ControlRow {
        kind: "menu",
        mac: "menu bar",
        windows: "MenuBar",
        gnome: "GMenu popover",
    },
    ControlRow {
        kind: "menu-item",
        mac: "menu item",
        windows: "MenuFlyoutItem",
        gnome: "GMenu item",
    },
    ControlRow {
        kind: "toolbar",
        mac: "toolbar",
        windows: "CommandBar",
        gnome: "header-bar actions",
    },
    ControlRow {
        kind: "tabs",
        mac: "tab view",
        windows: "TabView",
        gnome: "AdwViewSwitcher",
    },
    ControlRow {
        kind: "tab",
        mac: "tab view item",
        windows: "TabViewItem",
        gnome: "view switcher item",
    },
    ControlRow {
        kind: "settings",
        mac: "settings window form",
        windows: "settings page layout",
        gnome: "AdwPreferencesWindow",
    },
];

/// The control for a variant-driven kind on `platform` (#5852 §2,
/// the form-field rows): `field` by its `kind` attribute — text
/// entry variants, the checkbox variant, and the choice variant.
/// Returns `None` when the kind is not variant-driven or the
/// variant value has no row.
#[must_use]
pub fn variant_control(kind: &str, variant: Option<&str>, platform: &str) -> Option<&'static str> {
    let (mac, windows, gnome) = match (kind, variant) {
        ("field", Some("text" | "password" | "number" | "date" | "file")) => {
            ("text field", "TextBox", "GtkEntry")
        }
        ("field", Some("checkbox")) => ("checkbox", "CheckBox", "GtkCheckButton"),
        ("field", Some("choice")) => ("pop-up button", "ComboBox", "GtkDropDown"),
        _ => return None,
    };
    Some(match platform {
        "windows" => windows,
        "gnome" => gnome,
        _ => mac,
    })
}

/// Looks up one element kind in the control-selection table.
#[must_use]
pub fn control_row(kind: &str) -> Option<&'static ControlRow> {
    CONTROL_ROWS.iter().find(|r| r.kind == kind)
}

/// One assertion-role row of #5852 §3, keyed by the #5850
/// dimension (`property`) and the role token. Each platform entry
/// is the rendering the platform exposes for that role — a theme
/// key or standard-control form, named from the record — or
/// `None`, the explicit platform-default marker of §3.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleRow {
    /// Assertion dimension, e.g. `tone`.
    pub property: &'static str,
    /// Role token, e.g. `accent`.
    pub token: &'static str,
    /// macOS rendering, or `None` for platform-default.
    pub mac: Option<&'static str>,
    /// Windows rendering, or `None` for platform-default.
    pub windows: Option<&'static str>,
    /// GNOME rendering, or `None` for platform-default.
    pub gnome: Option<&'static str>,
}

impl RoleRow {
    /// The rendering this row gives on `platform`: `Some` is the
    /// mapping, `None` is the §3.1 platform-default marker.
    #[must_use]
    pub fn rendering_on(&self, platform: &str) -> Option<&'static str> {
        match platform {
            "windows" => self.windows,
            "gnome" => self.gnome,
            _ => self.mac,
        }
    }
}

/// #5852 §3.2 (tone), §3.3 (text), §3.4 (emphasis), §3.5
/// (density), §3.7 (corner/elevation). Spacing (§3.6) is
/// generative — `pad`/`gap` over the `space-0`…`space-8` keys —
/// and lives in [`spacing_rendering`].
pub const ROLE_ROWS: &[RoleRow] = &[
    // §3.2 tone — surface roles: system semantic background
    // colours / theme background resources / libadwaita
    // background variables, per container kind.
    RoleRow {
        property: "tone",
        token: "surface-base",
        mac: Some("system semantic background colour"),
        windows: Some("theme background resource"),
        gnome: Some("libadwaita background variable"),
    },
    RoleRow {
        property: "tone",
        token: "surface-raised",
        mac: Some("system semantic background colour"),
        windows: Some("theme background resource"),
        gnome: Some("libadwaita background variable"),
    },
    RoleRow {
        property: "tone",
        token: "surface-sunken",
        mac: Some("system semantic background colour"),
        windows: Some("theme background resource"),
        gnome: Some("libadwaita background variable"),
    },
    RoleRow {
        property: "tone",
        token: "surface-overlay",
        mac: Some("system semantic background colour"),
        windows: Some("theme layer resource"),
        gnome: Some("libadwaita popover background variable"),
    },
    RoleRow {
        property: "tone",
        token: "surface-inverse",
        mac: Some("system semantic background colour"),
        windows: Some("theme background resource"),
        gnome: Some("libadwaita background variable"),
    },
    // §3.2 tone — content roles.
    RoleRow {
        property: "tone",
        token: "content-primary",
        mac: Some("label colour"),
        windows: Some("primary text theme resource"),
        gnome: Some("foreground variable (full opacity)"),
    },
    RoleRow {
        property: "tone",
        token: "content-secondary",
        mac: Some("secondary label colour"),
        windows: Some("secondary text theme resource"),
        gnome: Some("foreground variable (dimmed opacity)"),
    },
    RoleRow {
        property: "tone",
        token: "content-muted",
        mac: Some("tertiary label colour"),
        windows: Some("secondary text theme resource"),
        gnome: Some("foreground variable (dimmed opacity)"),
    },
    RoleRow {
        property: "tone",
        token: "content-inverse",
        mac: Some("inverse label colour"),
        windows: Some("inverse text theme resource"),
        gnome: Some("inverse foreground variable"),
    },
    RoleRow {
        property: "tone",
        token: "content-link",
        mac: Some("link colour"),
        windows: Some("hyperlink resource"),
        gnome: Some("link variable"),
    },
    // §3.2 tone — accent pair.
    RoleRow {
        property: "tone",
        token: "accent",
        mac: Some("controlAccentColor"),
        windows: Some("Accent theme resource"),
        gnome: Some("accent background variable"),
    },
    RoleRow {
        property: "tone",
        token: "accent-content",
        mac: Some("platform content colour on accent"),
        windows: Some("on-accent theme resource"),
        gnome: Some("accent foreground variable"),
    },
    // §3.2 tone — status roles (colour; the surface tint rides
    // standard banner/message widgets where the control supports
    // them, per the record).
    RoleRow {
        property: "tone",
        token: "success",
        mac: Some("system green"),
        windows: Some("system success colour"),
        gnome: Some("success style class and variable"),
    },
    RoleRow {
        property: "tone",
        token: "warning",
        mac: Some("system yellow"),
        windows: Some("system warning colour"),
        gnome: Some("warning style class and variable"),
    },
    RoleRow {
        property: "tone",
        token: "danger",
        mac: Some("system red"),
        windows: Some("system danger colour"),
        gnome: Some("error style class and variable"),
    },
    RoleRow {
        property: "tone",
        token: "info",
        mac: Some("system blue"),
        windows: Some("system info colour"),
        gnome: Some("info style class and variable"),
    },
    // §3.2 tone — border roles. The focus ring is the platform's
    // own (§3.8), so border-focus is a platform-default entry on
    // every platform.
    RoleRow {
        property: "tone",
        token: "border-subtle",
        mac: Some("separator colour"),
        windows: Some("border theme resource"),
        gnome: Some("border variable"),
    },
    RoleRow {
        property: "tone",
        token: "border-strong",
        mac: Some("separator colour"),
        windows: Some("border theme resource"),
        gnome: Some("border variable"),
    },
    RoleRow {
        property: "tone",
        token: "border-focus",
        mac: None,
        windows: None,
        gnome: None,
    },
    // §3.3 text — semantic text styles per platform.
    RoleRow {
        property: "text",
        token: "display",
        mac: Some("Dynamic Type largeTitle"),
        windows: Some("Display text style resource"),
        gnome: Some("libadwaita title-1 class"),
    },
    RoleRow {
        property: "text",
        token: "heading",
        mac: Some("Dynamic Type headline"),
        windows: Some("Title text style resource"),
        gnome: Some("libadwaita title-2 class"),
    },
    RoleRow {
        property: "text",
        token: "body",
        mac: Some("Dynamic Type body"),
        windows: Some("Body text style resource"),
        gnome: Some("libadwaita body class"),
    },
    RoleRow {
        property: "text",
        token: "body-small",
        mac: Some("Dynamic Type subheadline"),
        windows: Some("Body small text style resource"),
        gnome: Some("libadwaita body class"),
    },
    RoleRow {
        property: "text",
        token: "caption",
        mac: Some("Dynamic Type caption"),
        windows: Some("Caption text style resource"),
        gnome: Some("libadwaita caption class"),
    },
    RoleRow {
        property: "text",
        token: "label",
        mac: Some("Dynamic Type body (label)"),
        windows: Some("Body text style resource (label)"),
        gnome: Some("libadwaita body class"),
    },
    RoleRow {
        property: "text",
        token: "code",
        mac: Some("Dynamic Type body (monospaced)"),
        windows: Some("monospaced theme font"),
        gnome: Some("libadwaita monospace class"),
    },
    RoleRow {
        property: "text",
        token: "link",
        mac: Some("Dynamic Type body (link)"),
        windows: Some("Body text style resource (link)"),
        gnome: Some("libadwaita body class (link)"),
    },
    // §3.4 emphasis — variant roles. Windows has no standard
    // destructive button style: destructive is a platform-default
    // entry for style there (its danger tone still maps, §3.2).
    RoleRow {
        property: "emphasis",
        token: "primary",
        mac: Some("default button with accent tint"),
        windows: Some("accent Button style"),
        gnome: Some("suggested-action style class"),
    },
    RoleRow {
        property: "emphasis",
        token: "destructive",
        mac: Some("destructive-role button"),
        windows: None,
        gnome: Some("destructive-action style class"),
    },
    RoleRow {
        property: "emphasis",
        token: "subtle",
        mac: Some("borderless button"),
        windows: Some("flat Button style"),
        gnome: Some("flat style class"),
    },
    // §3.5 density — control-size systems. GTK has no per-control
    // density system: density is platform-default on GNOME
    // (spacing roles §3.6 still apply).
    RoleRow {
        property: "density",
        token: "comfortable",
        mac: Some("regular control size"),
        windows: Some("standard density"),
        gnome: None,
    },
    RoleRow {
        property: "density",
        token: "compact",
        mac: Some("small/mini control size"),
        windows: Some("compact density"),
        gnome: None,
    },
    // §3.7 corner — the standard control's own shape wins on
    // every platform: every (corner, platform) pair is a
    // platform-default entry.
    RoleRow {
        property: "corner",
        token: "corner-square",
        mac: None,
        windows: None,
        gnome: None,
    },
    RoleRow {
        property: "corner",
        token: "corner-rounded",
        mac: None,
        windows: None,
        gnome: None,
    },
    RoleRow {
        property: "corner",
        token: "corner-full",
        mac: None,
        windows: None,
        gnome: None,
    },
    // §3.7 elevation — ordering only; the rendering mechanism is
    // the platform's layering.
    RoleRow {
        property: "elevation",
        token: "elevation-flat",
        mac: Some("window layering (flat)"),
        windows: Some("standard layering (flat)"),
        gnome: Some("border treatment (flat)"),
    },
    RoleRow {
        property: "elevation",
        token: "elevation-raised",
        mac: Some("material layering (raised)"),
        windows: Some("shadow layering (raised)"),
        gnome: Some("shadow treatment (raised)"),
    },
    RoleRow {
        property: "elevation",
        token: "elevation-overlay",
        mac: Some("sheet layering (overlay)"),
        windows: Some("acrylic layering (overlay)"),
        gnome: Some("shadow treatment (overlay)"),
    },
];

/// The rendering a spacing role gives on `platform` (#5852
/// §3.6): `pad`/`gap` over the theme keys `space-0`…`space-8`,
/// applied in the platform's layout unit — Mac points, Windows
/// effective pixels on its 4-unit grid, GNOME pixels on the
/// libadwaita spacing scale. The per-platform anchor values live
/// in the theme file; this names the role and the unit only.
/// `None` when the token is not a spacing key.
#[must_use]
pub fn spacing_rendering(property: &str, token: &str, platform: &str) -> Option<String> {
    if !matches!(property, "pad" | "gap") {
        return None;
    }
    let n: u32 = token.strip_prefix("space-")?.parse().ok()?;
    if n > 8 {
        return None;
    }
    let unit = match platform {
        "windows" => "effective pixels (4-unit grid)",
        "gnome" => "pixels (libadwaita spacing scale)",
        _ => "points",
    };
    Some(format!("space-{n} in {unit}"))
}

/// Looks up one assertion in the role table.
#[must_use]
pub fn role_row(property: &str, token: &str) -> Option<&'static RoleRow> {
    ROLE_ROWS
        .iter()
        .find(|r| r.property == property && r.token == token)
}
