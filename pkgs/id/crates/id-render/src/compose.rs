//! Stage 3 — compose (#5855 §2.1 stage 3), executing the composition
//! order settled by decision record #5854
//! (`doc/2026-11-08_decision_id5854_per_target_composition.md`).
//!
//! For one target, composition runs in two steps:
//!
//! 1. **Conditional blocks are resolved for the target** (#5854 §5:
//!    include/exclude *before* the cascade). A block is the
//!    `<conditional>` element (see [`crate::vocabulary::CONDITIONAL`]);
//!    its predicate is evaluated against the [`TargetSpec`] — exactly
//!    the target facts §5 allows (target name, form-factor class,
//!    declared capability flags) — by [`conditional_includes`]. An
//!    excluded block is dropped and recorded in the report's
//!    [`CompositionReport::conditional_exclusions`] (the #5855 §2.1
//!    stage-5 report lists exclusions; nothing is silent). An
//!    included block's children are spliced into the parent's
//!    children, so the resolved IR never holds a conditional node and
//!    the lowering stages need no case for one. Spliced children keep
//!    the block in their document path
//!    (`…/page[0]/conditional[0]/text[0]`), so a path names the same
//!    element on every target and the block's own index counts among
//!    its siblings whether or not it renders.
//! 2. **The #5854 cascade runs** (§2.1, low → high): platform
//!    defaults (layer 1, the lowering stage's concern — compose
//!    records nothing for it) → theme/token values (layer 2: values
//!    only, §3 layering base → target → mode variant) → semantic
//!    assertions (layer 3) → the per-target override map (layer 4:
//!    style overrides, then presentation overrides per #5853) →
//!    raw CSS (layer 5, web target only).
//!
//! Layer 3 has two inputs. The first is the source itself: each
//! element's `assert` slot, validated against the #5850
//! vocabulary by stage 2, enters as assertions at instance
//! specificity — the slot binds to this element instance in the
//! one source (#5854 §2.2). The second is *data* supplied by the
//! caller ([`ComposeData`]): further assertion sets, the
//! override map, and the theme tables. Elements are addressed by
//! their document path (see [`element_path`]), a deterministic
//! address derived from the IR itself — no source syntax is
//! invented for bindings. (#5855 assumption A2 carried the slot
//! opaquely until #5850 settled; the record has settled and the
//! deferral is discharged.)
//!
//! The output is the resolved IR for the one target
//! ([`ComposedDocument`]): every element carries its final
//! assertion set with the winning layer recorded per property,
//! values stay role-named (tokens) until lowering, exactly as
//! #5855 §2.1 stage 3 prescribes. The [`CompositionReport`]
//! carries what #5854 §6 makes obligatory: raw-CSS reported
//! overrides, warnings for unresolved tokens, ignored off-web raw
//! CSS, mode fallbacks, and unused override entries. Nothing is
//! silent (#5854 invariant 9); same-specificity ties are build
//! errors, never source-order resolutions (invariant 2).
//!
//! Raw CSS scoping in this slice: a `<raw kind="css">` region's
//! declarations are layer-5 candidates for the element that
//! *contains* the region, and the rule's selector text is carried
//! verbatim into the report for the web emitter, which owns the
//! platform's real selector cascade inside layer 5 (#5854 §2.2).
//! Compose does not implement CSS selector matching.

use std::collections::{BTreeMap, BTreeSet};

use crate::assertion_vocabulary;
use crate::{BuildError, Document, Element, Node, RawRegion, Span};

/// The facts about one target that composition may consult: the
/// target's name (the web target is the one named `web` — layer 5
/// exists only there, #5854 §2.1), its form-factor class, and its
/// declared capability flags. These are also exactly the facts a
/// conditional-block predicate may name (#5854 §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSpec {
    /// Target name, e.g. `web`, `gui`, `native`.
    pub name: String,
    /// Form-factor class, e.g. `desktop` or `handheld`.
    pub form_factor: String,
    /// Declared capability flags for this target.
    pub capabilities: BTreeSet<String>,
}

impl TargetSpec {
    /// Whether this is the web target — the only target where the
    /// raw-CSS layer (layer 5) applies (#5854 §2.1).
    #[must_use]
    pub fn is_web(&self) -> bool {
        self.name == "web"
    }
}

/// Binding specificity, shared by assertions (#5854 §2.2) and the
/// override map (#5853 §2.5): instance beats kind+variant+state
/// beats kind default. Declaration order in the input data is
/// never a tiebreaker (#5854 invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Specificity {
    /// A declaration for the element kind in general.
    KindDefault,
    /// A declaration for kind+variant+state.
    KindVariantState,
    /// A declaration bound to one element instance.
    Instance,
}

/// One semantic style assertion (layer 3), as data: on the element
/// at `element_path`, the assertion dimension `property` is set to
/// the named token `token` (roles/tokens only, never raw values —
/// #5854 invariant 3; a raw value here is the caller's contract
/// breach, caught when the token fails to resolve, §6c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionInput {
    /// Document path of the element (see [`element_path`]).
    pub element_path: String,
    /// Assertion dimension, e.g. `colour-role`.
    pub property: String,
    /// Named role/token the dimension is set to.
    pub token: String,
    /// Binding specificity.
    pub specificity: Specificity,
}

/// One entry of the per-target override map (layer 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideInput {
    /// A style override (layer 4a): adjusts one property of one
    /// element for this target only.
    Style {
        /// Document path of the element.
        element_path: String,
        /// Property (assertion dimension) being adjusted.
        property: String,
        /// Named token the property is adjusted to.
        token: String,
        /// Binding specificity (#5853 §2.5).
        specificity: Specificity,
    },
    /// A presentation override (layer 4b, #5853): replaces the
    /// element's rendering wholesale with the named presentation.
    /// When one applies, the element's assertions and style
    /// overrides no longer affect its appearance; they remain its
    /// fallback standard rendering (#5854 §2.1 layer 4).
    Presentation {
        /// Document path of the element.
        element_path: String,
        /// Name of the presentation (e.g. an asset pack name).
        presentation: String,
        /// Binding specificity (#5853 §2.5).
        specificity: Specificity,
    },
}

/// The theme tables for one target (#5854 §3): token name → value
/// in the target's idiom, in three layers. The mode variant is a
/// partial table: it overrides only the tokens it names, and a
/// token it does not name falls back to the target layer's value
/// (with a warning when the token is actually used, §3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeTables {
    /// Programme base theme.
    pub base: BTreeMap<String, String>,
    /// Per-target theme.
    pub target: BTreeMap<String, String>,
    /// Mode variant (e.g. `dark`), as (mode name, partial table).
    pub mode: Option<(String, BTreeMap<String, String>)>,
}

/// All data inputs to composition for one target, beyond the
/// validated document itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposeData {
    /// Layer-3 assertions, as data (see [`AssertionInput`]).
    pub assertions: Vec<AssertionInput>,
    /// Layer-4 override map entries for this target only
    /// (target locality, #5854 §2.4: the caller supplies one
    /// target's map; compose never sees another target's).
    pub overrides: Vec<OverrideInput>,
    /// Layer-2 theme tables for this target.
    pub theme: ThemeTables,
}

/// Which cascade layer supplied the winning declaration for one
/// property (#5854 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinningLayer {
    /// Layer 3, semantic assertions.
    Assertion,
    /// Layer 4a, style overrides.
    StyleOverride,
    /// Layer 5, raw CSS (web target only).
    RawCss,
}

/// The final resolved state of one property on one element: the
/// winning declaration, with the token still role-named (lowering
/// resolves values; compose records the theme-resolved value
/// alongside for the report and the emitter's convenience).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProperty {
    /// Assertion dimension / property name.
    pub property: String,
    /// The winning layer.
    pub layer: WinningLayer,
    /// The named token, for assertion/override winners.
    pub token: Option<String>,
    /// The literal value, for raw-CSS winners (layer 5 values are
    /// author CSS, the one place raw values may win, #5854 §2.1).
    pub raw_value: Option<String>,
    /// The theme-resolved value of `token` for this target, when
    /// the token resolves in the theme tables; `None` when it does
    /// not (a §6c warning is recorded in the report).
    pub theme_value: Option<String>,
}

/// One element of the resolved IR: mirrors the source element,
/// plus its composed state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedElement {
    /// Document path of this element.
    pub path: String,
    /// Element kind, as in the source IR.
    pub name: String,
    /// Source span of the element.
    pub span: Span,
    /// The winning presentation override's name, when one applies
    /// (layer 4b). The element's `properties` then describe its
    /// fallback standard rendering only (#5854 §2.1 layer 4).
    pub presentation: Option<String>,
    /// Final assertion set: one entry per property, sorted by
    /// property name (determinism, #5855 §5).
    pub properties: Vec<ResolvedProperty>,
    /// Composed children, in source order.
    pub children: Vec<ComposedNode>,
}

/// One node of the resolved IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposedNode {
    /// A composed element.
    Element(ComposedElement),
    /// Text content, carried unchanged (semantics are layer-proof,
    /// #5854 invariant 4).
    Text(String),
    /// A raw HTML region, carried unchanged. (Raw CSS regions are
    /// consumed by composition as layer-5 input and do not appear
    /// in the resolved IR.)
    RawHtml(RawRegion),
}

/// A raw-CSS declaration that won its property on the web target:
/// a *reported override* (#5854 §6b) — allowed, never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedOverride {
    /// Element the declaration applied to.
    pub element_path: String,
    /// Property it won.
    pub property: String,
    /// The rule's selector text, verbatim.
    pub selector: String,
    /// The winning literal value.
    pub value: String,
}

/// One build warning from composition (#5854 §6): a fallback,
/// an ignored rule, or dead configuration — reported, never
/// silent (invariant 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionWarning {
    /// What happened, naming the rule of #5854 §6 it comes from.
    pub message: String,
    /// Where in the source it belongs, when known.
    pub span: Option<Span>,
}

/// The composition report for one target (#5854 §6): a build
/// artefact, deterministic in content and order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompositionReport {
    /// Raw-CSS reported overrides (web target; §6b).
    pub reported_overrides: Vec<ReportedOverride>,
    /// Warnings (§6c/§6d/§6e and the §3 mode fallback).
    pub warnings: Vec<CompositionWarning>,
    /// Elements whose appearance is governed by a presentation
    /// override: (element path, presentation name).
    pub presentation_overrides: Vec<(String, String)>,
    /// Conditional blocks excluded for this target, in source order
    /// (#5854 §5, #5855 §2.1 stage 5).
    pub conditional_exclusions: Vec<ConditionalExclusion>,
}

/// One conditional block a target excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalExclusion {
    /// Document path of the `<conditional>` element.
    pub element_path: String,
    /// Where the block starts.
    pub span: Span,
    /// The block's predicate as written, e.g. `target="web"`.
    pub predicate: String,
}

/// Whether the conditional block `el` is included on `target`
/// (#5854 §5): every predicate attribute present must list a fact of
/// the target — its name (`target`), its form-factor class
/// (`form-factor`), or one of its declared capability flags
/// (`capability`). Within one attribute any listed name matches.
/// The predicate is total and deterministic: it reads target facts
/// only. A block with no predicate attribute fails validation, and is
/// treated as excluded here rather than as unconditional.
#[must_use]
pub fn conditional_includes(el: &Element, target: &TargetSpec) -> bool {
    let mut any = false;
    for attr in crate::vocabulary::PREDICATE_ATTRS {
        if !el.has_attr(attr) {
            continue;
        }
        any = true;
        let mut terms = el.attr(attr).unwrap_or("").split_whitespace();
        let hit = match *attr {
            "target" => terms.any(|t| t == target.name),
            "form-factor" => terms.any(|t| t == target.form_factor),
            _ => terms.any(|t| target.capabilities.contains(t)),
        };
        if !hit {
            return false;
        }
    }
    any
}

/// A conditional block's predicate as written, for reports:
/// `target="web" capability="pointer"`.
#[must_use]
pub fn predicate_text(el: &Element) -> String {
    crate::vocabulary::PREDICATE_ATTRS
        .iter()
        .filter_map(|attr| {
            el.has_attr(attr)
                .then(|| format!("{attr}=\"{}\"", el.attr(attr).unwrap_or("")))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The output of stage 3 for one target: the resolved IR plus
/// the composition report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedDocument {
    /// The target this document was composed for.
    pub target: String,
    /// The resolved root element (path `document`).
    pub root: ComposedElement,
    /// The composition report.
    pub report: CompositionReport,
}

/// The deterministic document path of the element at
/// `sibling_index` (its index among its same-kind siblings) under
/// `parent_path`: e.g. `document/page[0]/form[0]/button[1]`.
/// This is the address composition data binds to; it is derived
/// from the IR alone, so it is stable for a given source document.
#[must_use]
pub fn element_path(parent_path: &str, kind: &str, sibling_index: usize) -> String {
    format!("{parent_path}/{kind}[{sibling_index}]")
}

/// One raw-CSS declaration parsed from a `<raw kind="css">`
/// region: selector text verbatim, property, value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CssDeclaration {
    selector: String,
    property: String,
    value: String,
}

/// Parses raw CSS into declarations, best-effort at rule
/// granularity: `selector { property: value; ... }`. Fragments
/// that do not parse are returned as the second element so the
/// caller can warn — malformed CSS is never silently dropped
/// (#5854 invariant 9).
fn parse_css(content: &str) -> (Vec<CssDeclaration>, Vec<String>) {
    let mut declarations = Vec::new();
    let mut malformed = Vec::new();
    for chunk in content.split('}') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let Some((selector, body)) = chunk.split_once('{') else {
            malformed.push(chunk.to_string());
            continue;
        };
        let selector = selector.trim().to_string();
        if selector.is_empty() {
            malformed.push(chunk.to_string());
            continue;
        }
        for decl in body.split(';') {
            let decl = decl.trim();
            if decl.is_empty() {
                continue;
            }
            match decl.split_once(':') {
                Some((property, value))
                    if !property.trim().is_empty() && !value.trim().is_empty() =>
                {
                    declarations.push(CssDeclaration {
                        selector: selector.clone(),
                        property: property.trim().to_string(),
                        value: value.trim().to_string(),
                    });
                }
                _ => malformed.push(format!("{selector} {{ {decl} }}")),
            }
        }
    }
    (declarations, malformed)
}

/// Resolves one token through the theme layering (base → target
/// → mode variant, #5854 §3). Returns the value and which layer
/// supplied it; a token the mode variant does not name falls back
/// to the target/base value (the caller warns, §3).
fn resolve_token(theme: &ThemeTables, token: &str) -> Option<(String, &'static str)> {
    if let Some((_, mode)) = &theme.mode {
        if let Some(v) = mode.get(token) {
            return Some((v.clone(), "mode"));
        }
    }
    if let Some(v) = theme.target.get(token) {
        return Some((v.clone(), "target"));
    }
    theme.base.get(token).map(|v| (v.clone(), "base"))
}

/// Whether the mode variant names `token` (used for the §3
/// mode-fallback warning: the variant is partial, so absence is
/// normal — the warning fires only for tokens a winning
/// declaration actually uses).
fn mode_names(theme: &ThemeTables, token: &str) -> bool {
    theme
        .mode
        .as_ref()
        .is_some_and(|(_, m)| m.contains_key(token))
}

/// Picks the winner among same-layer declarations for one
/// property: highest specificity wins; two *different* tokens at
/// the highest specificity are a same-specificity tie — a build
/// error (#5854 §6a), never a source-order resolution. Identical
/// redeclarations of the same token dedupe silently (they are
/// the same declaration, not a tie).
fn pick_winner<'a, I>(
    candidates: I,
    path: &str,
    property: &str,
    span: Span,
    layer_name: &str,
) -> Result<Option<&'a (String, Specificity)>, BuildError>
where
    I: Iterator<Item = &'a (String, Specificity)>,
{
    let all: Vec<&(String, Specificity)> = candidates.collect();
    let Some(top) = all.iter().map(|c| c.1).max() else {
        return Ok(None);
    };
    let mut at_top = all.iter().copied().filter(|c| c.1 == top);
    let first = at_top.next();
    if let Some(first) = first
        && let Some(rival) = at_top.find(|c| c.0 != first.0)
    {
        return Err(BuildError::new(
            format!(
                "same-specificity tie on <{path}> property `{property}` in {layer_name}: `{}` vs `{}` — ties are build errors, never resolved by declaration order (#5854 §6a)",
                first.0, rival.0
            ),
            span.line,
            span.col,
        ));
    }
    Ok(first)
}

/// Stage 3 of the #5855 pipeline: compose a validated document
/// for one target, executing the #5854 cascade over the supplied
/// data, after resolving conditional blocks for the target (#5854
/// §5). See the module documentation.
///
/// # Errors
///
/// Returns every same-specificity tie found (each located at the
/// element it binds), or the composed document.
pub fn compose(
    doc: &Document,
    target: &TargetSpec,
    data: &ComposeData,
) -> Result<ComposedDocument, Vec<BuildError>> {
    let mut cx = ComposeCx {
        target,
        data,
        report: CompositionReport::default(),
        errors: Vec::new(),
        element_paths: BTreeSet::new(),
    };
    let root = cx.compose_element(&doc.root, "document");
    // §6e: data entries that bind no element in the document are
    // dead configuration for this target — warn, never silent.
    for entry in &data.overrides {
        let path = match entry {
            OverrideInput::Style { element_path, .. }
            | OverrideInput::Presentation { element_path, .. } => element_path,
        };
        if !cx.element_paths.contains(path) {
            cx.report.warnings.push(CompositionWarning {
                message: format!(
                    "override entry for `{path}` binds no element in this document — unused override entry (#5854 §6e)"
                ),
                span: None,
            });
        }
    }
    for entry in &data.assertions {
        if !cx.element_paths.contains(&entry.element_path) {
            cx.report.warnings.push(CompositionWarning {
                message: format!(
                    "assertion for `{}` binds no element in this document — unused assertion entry (#5854 §6e)",
                    entry.element_path
                ),
                span: None,
            });
        }
    }
    if cx.errors.is_empty() {
        Ok(ComposedDocument {
            target: target.name.clone(),
            root,
            report: cx.report,
        })
    } else {
        Err(cx.errors)
    }
}

/// Working state for one composition pass.
struct ComposeCx<'a> {
    target: &'a TargetSpec,
    data: &'a ComposeData,
    report: CompositionReport,
    errors: Vec<BuildError>,
    element_paths: BTreeSet<String>,
}

impl ComposeCx<'_> {
    /// Composes one element and, recursively, its children.
    fn compose_element(&mut self, el: &Element, path: &str) -> ComposedElement {
        self.element_paths.insert(path.to_string());
        let mut by_property: BTreeMap<String, ResolvedProperty> = BTreeMap::new();

        // Layer 3: assertions for this element — the element's
        // own `assert` slot at instance specificity, plus the
        // caller-supplied data assertions bound to this path.
        // The slot was validated by stage 2; a document that
        // reaches compose unvalidated and carries a bad slot
        // fails here with the same located errors rather than
        // silently dropping the assertions.
        let mut source_assertions: Vec<AssertionInput> = Vec::new();
        if el.has_attr("assert") {
            match el.attr("assert") {
                Some(value) => match assertion_vocabulary::parse_assert_slot(value) {
                    Ok(pairs) => {
                        source_assertions.extend(pairs.into_iter().map(|pair| AssertionInput {
                            element_path: path.to_string(),
                            property: pair.dimension,
                            token: pair.token,
                            specificity: Specificity::Instance,
                        }));
                    }
                    Err(problems) => {
                        for problem in problems {
                            self.errors
                                .push(BuildError::new(problem, el.span.line, el.span.col));
                        }
                    }
                },
                None => {
                    self.errors.push(BuildError::new(
                        format!("attribute `assert` on <{}> requires a value", el.name),
                        el.span.line,
                        el.span.col,
                    ));
                }
            }
        }
        let assertions: Vec<&AssertionInput> = self
            .data
            .assertions
            .iter()
            .filter(|a| a.element_path == path)
            .chain(source_assertions.iter())
            .collect();
        let mut properties: BTreeSet<String> = BTreeSet::new();
        for a in &assertions {
            properties.insert(a.property.clone());
        }
        // Layer 4a: style overrides for this element.
        let style_overrides: Vec<&OverrideInput> = self
            .data
            .overrides
            .iter()
            .filter(
                |o| matches!(o, OverrideInput::Style { element_path, .. } if element_path == path),
            )
            .collect();
        for o in &style_overrides {
            if let OverrideInput::Style { property, .. } = o {
                properties.insert(property.clone());
            }
        }

        // Layer 5 input: raw CSS regions among this element's
        // children, parsed once here (consumed below, and not
        // carried into the composed children). In the IR a raw
        // region is a `<raw>` element holding one Raw node; a
        // `kind="css"` region among this element's children is
        // layer-5 input for this element.
        let mut raw_css: Vec<CssDeclaration> = Vec::new();
        let included = self.included_children(el, path);
        for (child, _) in &included {
            let Node::Element(raw_el) = child else {
                continue;
            };
            if raw_el.name != "raw" || raw_el.attr("kind") != Some("css") {
                continue;
            }
            let Some(Node::Raw(raw)) = raw_el.children.first() else {
                continue;
            };
            let (decls, malformed) = parse_css(&raw.content);
            raw_css.extend(decls);
            for fragment in malformed {
                self.report.warnings.push(CompositionWarning {
                    message: format!(
                        "unparseable raw CSS fragment in <{path}> ignored: `{fragment}`"
                    ),
                    span: Some(raw.span),
                });
            }
            if !self.target.is_web() && !raw.content.trim().is_empty() {
                // §2.3/§6d: raw CSS is never translated off-web;
                // it is ignored, once per region.
                self.report.warnings.push(CompositionWarning {
                    message: format!(
                        "raw CSS on target `{}` is ignored (never translated to a non-web idiom) — region in <{path}> (#5854 §2.3)",
                        self.target.name
                    ),
                    span: Some(raw.span),
                });
            }
        }
        if self.target.is_web() {
            for decl in &raw_css {
                properties.insert(decl.property.clone());
            }
        }

        for property in &properties {
            let assertion_cands: Vec<(String, Specificity)> = assertions
                .iter()
                .filter(|a| &a.property == property)
                .map(|a| (a.token.clone(), a.specificity))
                .collect();
            let override_cands: Vec<(String, Specificity)> = style_overrides
                .iter()
                .filter_map(|o| match o {
                    OverrideInput::Style {
                        property: p,
                        token,
                        specificity,
                        ..
                    } if p == property => Some((token.clone(), *specificity)),
                    _ => None,
                })
                .collect();
            let assertion_win = match pick_winner(
                assertion_cands.iter(),
                path,
                property,
                el.span,
                "semantic assertions",
            ) {
                Ok(w) => w.cloned(),
                Err(e) => {
                    self.errors.push(e);
                    None
                }
            };
            let override_win = match pick_winner(
                override_cands.iter(),
                path,
                property,
                el.span,
                "the override map",
            ) {
                Ok(w) => w.cloned(),
                Err(e) => {
                    self.errors.push(e);
                    None
                }
            };
            // Cascade: layer 4a beats layer 3 (#5854 §2.1).
            let mut winner: Option<ResolvedProperty> = None;
            if let Some((token, _)) = assertion_win {
                winner = Some(self.token_property(property, WinningLayer::Assertion, &token, el));
            }
            if let Some((token, _)) = override_win {
                winner =
                    Some(self.token_property(property, WinningLayer::StyleOverride, &token, el));
            }
            // Layer 5 (web only): a raw-CSS declaration for this
            // property on this element wins per property and is a
            // reported override when it displaces an
            // assertion-derived value (§6b). Inside layer 5 the
            // web platform's own cascade applies, including
            // source-order resolution — the last declaration of
            // the property wins (§2.2).
            if self.target.is_web() {
                if let Some(decl) = raw_css.iter().filter(|d| &d.property == property).last() {
                    if winner.is_some() {
                        self.report.reported_overrides.push(ReportedOverride {
                            element_path: path.to_string(),
                            property: property.clone(),
                            selector: decl.selector.clone(),
                            value: decl.value.clone(),
                        });
                    }
                    winner = Some(ResolvedProperty {
                        property: property.clone(),
                        layer: WinningLayer::RawCss,
                        token: None,
                        raw_value: Some(decl.value.clone()),
                        theme_value: None,
                    });
                }
            }
            if let Some(w) = winner {
                by_property.insert(property.clone(), w);
            }
        }

        // Layer 4b: presentation overrides (#5853 specificity;
        // ties are errors). A winner supersedes layers 3–4a for
        // appearance; the resolved properties stay as the
        // fallback standard rendering.
        let presentation_cands: Vec<(String, Specificity)> = self
            .data
            .overrides
            .iter()
            .filter_map(|o| match o {
                OverrideInput::Presentation {
                    element_path,
                    presentation,
                    specificity,
                } if element_path == path => Some((presentation.clone(), *specificity)),
                _ => None,
            })
            .collect();
        let mut presentation = None;
        if !presentation_cands.is_empty() {
            match pick_winner(
                presentation_cands.iter(),
                path,
                "presentation",
                el.span,
                "the override map",
            ) {
                Ok(Some((name, _))) => {
                    presentation = Some(name.clone());
                    self.report
                        .presentation_overrides
                        .push((path.to_string(), name.clone()));
                }
                Ok(None) => {}
                Err(e) => self.errors.push(e),
            }
        }

        // Children (conditional blocks already resolved for the
        // target by `included_children`).
        let mut children = Vec::new();
        for (child, child_path) in included {
            match child {
                Node::Element(child_el)
                    if child_el.name == "raw" && child_el.attr("kind") == Some("css") =>
                {
                    // Consumed above as this element's layer-5
                    // input; not part of the resolved IR.
                }
                Node::Element(child_el) => {
                    let child_path = child_path.unwrap_or_default();
                    children.push(ComposedNode::Element(
                        self.compose_element(child_el, &child_path),
                    ));
                }
                Node::Text(text) => children.push(ComposedNode::Text(text.clone())),
                Node::Raw(raw) => {
                    if raw.kind != "css" {
                        children.push(ComposedNode::RawHtml(raw.clone()));
                    }
                }
            }
        }

        ComposedElement {
            path: path.to_string(),
            name: el.name.clone(),
            span: el.span,
            presentation,
            properties: by_property.into_values().collect(),
            children,
        }
    }

    /// The children of `el` that render on this target, each with
    /// its document path (elements only): per-kind sibling indices,
    /// conditional blocks counted among their siblings, an included
    /// block's children spliced in under the block's path, and an
    /// excluded block recorded in the report (#5854 §5).
    fn included_children<'a>(
        &mut self,
        el: &'a Element,
        path: &str,
    ) -> Vec<(&'a Node, Option<String>)> {
        let mut out = Vec::new();
        let mut kind_counts: BTreeMap<&str, usize> = BTreeMap::new();
        for child in &el.children {
            let Node::Element(child_el) = child else {
                out.push((child, None));
                continue;
            };
            let idx = {
                let n = kind_counts.entry(child_el.name.as_str()).or_insert(0);
                let i = *n;
                *n += 1;
                i
            };
            let child_path = element_path(path, &child_el.name, idx);
            if child_el.name != crate::vocabulary::CONDITIONAL {
                out.push((child, Some(child_path)));
                continue;
            }
            if conditional_includes(child_el, self.target) {
                out.extend(self.included_children(child_el, &child_path));
            } else if !self
                .report
                .conditional_exclusions
                .iter()
                .any(|e| e.element_path == child_path)
            {
                self.report
                    .conditional_exclusions
                    .push(ConditionalExclusion {
                        element_path: child_path,
                        span: child_el.span,
                        predicate: predicate_text(child_el),
                    });
            }
        }
        out
    }

    /// Builds the resolved property for a token winner, resolving
    /// the token through the theme and recording the §6c / §3
    /// warnings that belong to it.
    fn token_property(
        &mut self,
        property: &str,
        layer: WinningLayer,
        token: &str,
        el: &Element,
    ) -> ResolvedProperty {
        let theme_value = match resolve_token(&self.data.theme, token) {
            Some((value, layer_name)) => {
                if layer_name != "mode"
                    && self.data.theme.mode.is_some()
                    && !mode_names(&self.data.theme, token)
                {
                    if let Some((mode_name, _)) = &self.data.theme.mode {
                        self.report.warnings.push(CompositionWarning {
                            message: format!(
                                "mode variant `{mode_name}` supplies no value for token `{token}`; fell back to the {layer_name} theme value (#5854 §3)"
                            ),
                            span: Some(el.span),
                        });
                    }
                }
                Some(value)
            }
            None => {
                self.report.warnings.push(CompositionWarning {
                    message: format!(
                        "token `{token}` (property `{property}` on <{}>) has no value in any theme layer for target `{}`; it falls back to the layer default at lowering (#5854 §6c)",
                        el.name, self.target.name
                    ),
                    span: Some(el.span),
                });
                None
            }
        };
        ResolvedProperty {
            property: property.to_string(),
            layer,
            token: Some(token.to_string()),
            raw_value: None,
            theme_value,
        }
    }
}
